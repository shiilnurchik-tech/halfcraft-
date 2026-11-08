use super::*;
use std::{
    ffi::c_void,
    ptr::NonNull,
    sync::atomic::{AtomicU32, AtomicU64, Ordering},
};

type Handle = *mut c_void;
const FILE_MAP_READ: u32 = 4;
const FILE_MAP_WRITE: u32 = 2;
#[link(name = "kernel32")]
unsafe extern "system" {
    fn OpenFileMappingW(access: u32, inherit: i32, name: *const u16) -> Handle;
    fn MapViewOfFile(
        mapping: Handle,
        access: u32,
        high: u32,
        low: u32,
        bytes: usize,
    ) -> *mut c_void;
    fn UnmapViewOfFile(base: *const c_void) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
    fn GetTickCount64() -> u64;
    fn GetCurrentProcessId() -> u32;
}

pub struct Mapping {
    handle: Handle,
    base: NonNull<u32>,
    owner: Option<(u32, u64)>,
    input_tail: u64,
}
// OS views/handles can cross threads. The enclosing Mutex serializes native
// callbacks; no reference into the view escapes this module.
unsafe impl Send for Mapping {}

impl Mapping {
    pub fn open() -> Result<Self, LinkError> {
        Self::open_access(false)
    }
    /// Exclusive input consumer. Never sets MC_IN_WORLD before the renderer and
    /// collider are ready. A running Fabric/other client is rejected, not evicted.
    pub fn connect() -> Result<Self, LinkError> {
        Self::open_access(true)
    }
    fn open_access(writable: bool) -> Result<Self, LinkError> {
        Self::open_named(MAPPING_NAME, writable)
    }
    fn open_named(name: &str, writable: bool) -> Result<Self, LinkError> {
        let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let access = FILE_MAP_READ | if writable { FILE_MAP_WRITE } else { 0 };
        // SAFETY: nul terminated string lives throughout this OS call.
        let handle = unsafe { OpenFileMappingW(access, 0, name.as_ptr()) };
        if handle.is_null() {
            return Err(LinkError::Unavailable);
        }
        let base = unsafe { MapViewOfFile(handle, access, 0, 0, VIEW_BYTES) };
        let Some(base) = NonNull::new(base.cast::<u32>()) else {
            unsafe {
                CloseHandle(handle);
            }
            return Err(LinkError::Unavailable);
        };
        let mut mapping = Self {
            handle,
            base,
            owner: None,
            input_tail: 0,
        };
        let state = mapping.read()?;
        if writable {
            let pid = unsafe { GetCurrentProcessId() };
            // Header uses the same naturally aligned atomic u32 as Source.
            mapping
                .atomic32(HEADER_MC_PID)
                .compare_exchange(0, pid, Ordering::AcqRel, Ordering::Acquire)
                .map_err(|_| LinkError::ClientAlreadyConnected)?;
            mapping.owner = Some((pid, state.session));
            mapping.check_owner()?;
            // Do not inherit stale state from a former consumer.
            mapping.clear_ready();
            mapping.input_tail = mapping
                .atomic64(INPUT_RING_OFFSET + INPUT_RING_HEAD_OFF as usize)
                .load(Ordering::Acquire);
            mapping
                .atomic64(INPUT_RING_OFFSET + INPUT_RING_TAIL_OFF as usize)
                .store(mapping.input_tail, Ordering::Release);
            mapping
                .atomic64(HEADER_MC_HEARTBEAT_MS)
                .store(Self::now_ms(), Ordering::Release);
        }
        Ok(mapping)
    }
    fn atomic32(&self, at: usize) -> &AtomicU32 {
        assert!(at % 4 == 0 && at + 4 <= VIEW_BYTES);
        // SAFETY: page-aligned view, validated field offset and mapping lifetime.
        unsafe { &*self.base.as_ptr().add(at / 4).cast::<AtomicU32>() }
    }
    fn atomic64(&self, at: usize) -> &AtomicU64 {
        assert!(at % 8 == 0 && at + 8 <= VIEW_BYTES);
        unsafe { &*self.base.as_ptr().add(at / 4).cast::<AtomicU64>() }
    }
    fn word(&self, at: usize) -> u32 {
        assert!(at % 4 == 0 && at + 4 <= VIEW_BYTES);
        unsafe { self.base.as_ptr().add(at / 4).read_volatile() }
    }
    fn check_owner(&self) -> Result<(), LinkError> {
        let Some((pid, session)) = self.owner else {
            return Err(LinkError::OwnershipLost);
        };
        if self.atomic32(HEADER_MC_PID).load(Ordering::Acquire) != pid
            || self.atomic64(HEADER_HOST_SESSION).load(Ordering::Acquire) != session
        {
            return Err(LinkError::OwnershipLost);
        }
        Ok(())
    }
    fn clear_ready(&self) {
        let seq = self.atomic32(MC_STATE_OFFSET + MC_STATE_SEQ);
        let previous = seq.load(Ordering::Acquire) & !1;
        seq.store(previous.wrapping_add(1), Ordering::SeqCst);
        self.atomic32(MC_STATE_OFFSET + MC_STATE_FLAGS)
            .store(0, Ordering::Relaxed);
        seq.store(previous.wrapping_add(2), Ordering::Release);
    }
    pub fn now_ms() -> u64 {
        unsafe { GetTickCount64() }
    }
    pub fn read(&self) -> Result<HostSnapshot, LinkError> {
        if self.owner.is_some() {
            self.check_owner()?;
        }
        let state = snapshot(|at| self.word(at), Self::now_ms())?;
        if self
            .owner
            .is_some_and(|(_, session)| session != state.session)
        {
            return Err(LinkError::OwnershipLost);
        }
        Ok(state)
    }
    pub fn drain_input(&mut self) -> Result<Vec<input::InputEvent>, LinkError> {
        self.read()?;
        let head = self
            .atomic64(INPUT_RING_OFFSET + INPUT_RING_HEAD_OFF as usize)
            .load(Ordering::Acquire);
        let (events, tail) = read_input_batch(head, self.input_tail, |at| self.word(at))?;
        self.check_owner()?;
        self.atomic64(INPUT_RING_OFFSET + INPUT_RING_TAIL_OFF as usize)
            .store(tail, Ordering::Release);
        self.atomic64(HEADER_MC_HEARTBEAT_MS)
            .store(Self::now_ms(), Ordering::Release);
        self.input_tail = tail;
        Ok(events)
    }
}
impl Drop for Mapping {
    fn drop(&mut self) {
        if self.owner.is_some() && self.check_owner().is_ok() {
            self.clear_ready();
            self.atomic64(HEADER_MC_HEARTBEAT_MS)
                .store(0, Ordering::Release);
        }
        // A session reset invalidates the old state, but may leave our claimed
        // pid behind if it raced the initial claim. Release only our own pid;
        // never clear another client's state/heartbeat after ownership loss.
        if let Some((pid, _)) = self.owner {
            let _ = self.atomic32(HEADER_MC_PID).compare_exchange(
                pid,
                0,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
        }
        unsafe {
            UnmapViewOfFile(self.base.as_ptr().cast());
            CloseHandle(self.handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    static NUMBER: AtomicUsize = AtomicUsize::new(0);
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateFileMappingW(
            file: Handle,
            attributes: *const c_void,
            protect: u32,
            high: u32,
            low: u32,
            name: *const u16,
        ) -> Handle;
    }
    struct HostView {
        handle: Handle,
        base: *mut u32,
        name: String,
    }
    impl HostView {
        fn new() -> Self {
            let name = format!(
                "Local\\HalfCraft_Test_{}_{}",
                unsafe { GetCurrentProcessId() },
                NUMBER.fetch_add(1, Ordering::Relaxed)
            );
            let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
            let handle = unsafe {
                CreateFileMappingW(
                    (-1isize) as Handle,
                    std::ptr::null(),
                    4,
                    0,
                    VIEW_BYTES as u32,
                    wide.as_ptr(),
                )
            };
            assert!(!handle.is_null());
            let base =
                unsafe { MapViewOfFile(handle, FILE_MAP_READ | FILE_MAP_WRITE, 0, 0, VIEW_BYTES) }
                    .cast::<u32>();
            assert!(!base.is_null());
            let host = Self { handle, base, name };
            host.set(HEADER_MAGIC, MAGIC);
            host.set(HEADER_LAYOUT, LAYOUT);
            host.set64(HEADER_HOST_SESSION, 77);
            host.set64(HEADER_HOST_HEARTBEAT_MS, Mapping::now_ms());
            host
        }
        fn set(&self, at: usize, value: u32) {
            unsafe {
                self.base.add(at / 4).write_volatile(value);
            }
        }
        fn get(&self, at: usize) -> u32 {
            unsafe { self.base.add(at / 4).read_volatile() }
        }
        fn set64(&self, at: usize, value: u64) {
            self.set(at, value as u32);
            self.set(at + 4, (value >> 32) as u32);
        }
    }
    impl Drop for HostView {
        fn drop(&mut self) {
            unsafe {
                UnmapViewOfFile(self.base.cast());
                CloseHandle(self.handle);
            }
        }
    }
    #[test]
    fn real_mapping_claim_drain_and_release() {
        let host = HostView::new();
        let mut client = Mapping::open_named(&host.name, true).unwrap();
        assert_ne!(host.get(HEADER_MC_PID), 0);
        assert_eq!(host.get(MC_STATE_OFFSET + MC_STATE_FLAGS), 0);
        assert!(matches!(
            Mapping::open_named(&host.name, true),
            Err(LinkError::ClientAlreadyConnected)
        ));
        let at = INPUT_RING_OFFSET + INPUT_RING_DATA_OFF as usize;
        host.set(at, IN_KEY | (26 << 16));
        host.set(at + 4, 1);
        host.set64(INPUT_RING_OFFSET + INPUT_RING_HEAD_OFF as usize, 1);
        let events = client.drain_input().unwrap();
        assert_eq!(
            events,
            vec![input::InputEvent {
                kind: IN_KEY as u16,
                code: 26,
                values: [1, 0, 0]
            }]
        );
        assert_eq!(
            host.get(INPUT_RING_OFFSET + INPUT_RING_TAIL_OFF as usize),
            1
        );
        assert!(client.drain_input().unwrap().is_empty());
        drop(client);
        assert_eq!(host.get(HEADER_MC_PID), 0);
        assert_eq!(host.get(HEADER_MC_HEARTBEAT_MS), 0);
        assert_eq!(host.get(MC_STATE_OFFSET + MC_STATE_FLAGS), 0);
    }
    #[test]
    fn ownership_loss_does_not_clobber_replacement_client() {
        let host = HostView::new();
        let client = Mapping::open_named(&host.name, true).unwrap();
        host.set(HEADER_MC_PID, 4242);
        host.set(MC_STATE_OFFSET + MC_STATE_FLAGS, 99);
        host.set64(HEADER_MC_HEARTBEAT_MS, 12345);
        assert_eq!(client.read(), Err(LinkError::OwnershipLost));
        drop(client);
        assert_eq!(host.get(HEADER_MC_PID), 4242);
        assert_eq!(host.get(HEADER_MC_HEARTBEAT_MS), 12345);
        assert_eq!(host.get(MC_STATE_OFFSET + MC_STATE_FLAGS), 99);
    }
    #[test]
    fn observer_never_claims_or_mutates_mapping() {
        let host = HostView::new();
        host.set(HEADER_MC_PID, 4242);
        let client = Mapping::open_named(&host.name, false).unwrap();
        assert_eq!(client.read().unwrap().session, 77);
        drop(client);
        assert_eq!(host.get(HEADER_MC_PID), 4242);
    }
}
