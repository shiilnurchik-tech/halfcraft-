//! Source transport with an optional exclusive input-consumer lease. No readiness:
//! until rendering/collision are ported, Source must retain player authority.
pub mod generated;
pub mod input;
#[cfg(windows)]
mod windows;
use generated::*;

pub const VIEW_BYTES: usize = INPUT_RING_OFFSET + INPUT_RING_SIZE;
const HOST_TIMEOUT_MS: u64 = 8_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkError {
    UnsupportedPlatform,
    Unavailable,
    Incompatible,
    Stale,
    Busy,
    InvalidState,
    ClientAlreadyConnected,
    OwnershipLost,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HostSnapshot {
    pub session: u64,
    pub flags: u32,
    pub world_id: u32,
    pub collision_epoch: u32,
    pub position: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub teleport_sequence: u32,
}

// The reader supplies aligned volatile u32 loads on Windows. No Rust references
// to concurrently written foreign memory escape the mapping, and no bytes are
// written. Source's existing seqlock is the external-process synchronization ABI.
fn snapshot(mut read: impl FnMut(usize) -> u32, now: u64) -> Result<HostSnapshot, LinkError> {
    fn u64_at(read: &mut impl FnMut(usize) -> u32, at: usize) -> u64 {
        u64::from(read(at)) | (u64::from(read(at + 4)) << 32)
    }
    if read(HEADER_MAGIC) != MAGIC || read(HEADER_LAYOUT) != LAYOUT {
        return Err(LinkError::Incompatible);
    }
    let session = u64_at(&mut read, HEADER_HOST_SESSION);
    let heartbeat = u64_at(&mut read, HEADER_HOST_HEARTBEAT_MS);
    if session == 0 || heartbeat == 0 || heartbeat > now || now - heartbeat > HOST_TIMEOUT_MS {
        return Err(LinkError::Stale);
    }
    // Bounded retries: a stalled writer must never stall Horizon's tick thread.
    for _ in 0..3 {
        let seq = read(HOST_STATE_OFFSET + HOST_STATE_SEQ);
        if seq & 1 != 0 {
            continue;
        }
        std::sync::atomic::fence(std::sync::atomic::Ordering::SeqCst);
        let mut words = [0u32; HOST_STATE_SIZE / 4];
        for (i, word) in words.iter_mut().enumerate() {
            *word = read(HOST_STATE_OFFSET + i * 4);
        }
        std::sync::atomic::fence(std::sync::atomic::Ordering::SeqCst);
        if read(HOST_STATE_OFFSET + HOST_STATE_SEQ) != seq {
            continue;
        }
        if read(HEADER_MAGIC) != MAGIC
            || read(HEADER_LAYOUT) != LAYOUT
            || u64_at(&mut read, HEADER_HOST_SESSION) != session
        {
            return Err(LinkError::Stale);
        }
        let word = |offset: usize| words[offset / 4];
        let double = |offset: usize| {
            f64::from_bits(u64::from(word(offset)) | (u64::from(word(offset + 4)) << 32))
        };
        let state = HostSnapshot {
            session,
            flags: word(HOST_STATE_FLAGS),
            world_id: word(HOST_STATE_WORLD_ID),
            collision_epoch: word(HOST_STATE_COLLISION_EPOCH),
            position: [
                double(HOST_STATE_POS_X),
                double(HOST_STATE_POS_Y),
                double(HOST_STATE_POS_Z),
            ],
            yaw: f32::from_bits(word(HOST_STATE_YAW)),
            pitch: f32::from_bits(word(HOST_STATE_PITCH)),
            teleport_sequence: word(HOST_STATE_TELEPORT_SEQ),
        };
        if !state.position.iter().all(|v| v.is_finite())
            || !state.yaw.is_finite()
            || !state.pitch.is_finite()
        {
            return Err(LinkError::InvalidState);
        }
        return Ok(state);
    }
    Err(LinkError::Busy)
}

#[cfg(windows)]
pub use windows::Mapping;

#[cfg(not(windows))]
pub struct Mapping;
#[cfg(not(windows))]
impl Mapping {
    pub fn open() -> Result<Self, LinkError> {
        Err(LinkError::UnsupportedPlatform)
    }
    pub fn read(&self) -> Result<HostSnapshot, LinkError> {
        Err(LinkError::UnsupportedPlatform)
    }
    pub fn connect() -> Result<Self, LinkError> {
        Err(LinkError::UnsupportedPlatform)
    }
    pub fn drain_input(&mut self) -> Result<Vec<input::InputEvent>, LinkError> {
        Err(LinkError::UnsupportedPlatform)
    }
    pub fn now_ms() -> u64 {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> [u32; VIEW_BYTES / 4] {
        let mut words = [0; VIEW_BYTES / 4];
        words[HEADER_MAGIC / 4] = MAGIC;
        words[HEADER_LAYOUT / 4] = LAYOUT;
        words[HEADER_HOST_SESSION / 4] = 7;
        words[HEADER_HOST_HEARTBEAT_MS / 4] = 100;
        let bits = 123.5f64.to_bits();
        words[(HOST_STATE_OFFSET + HOST_STATE_POS_X) / 4] = bits as u32;
        words[(HOST_STATE_OFFSET + HOST_STATE_POS_X + 4) / 4] = (bits >> 32) as u32;
        words[(HOST_STATE_OFFSET + HOST_STATE_WORLD_ID) / 4] = 42;
        words
    }
    #[test]
    fn compatible_source_layout_decodes() {
        let words = fixture();
        let state = snapshot(|at| words[at / 4], 200).unwrap();
        assert_eq!(state.session, 7);
        assert_eq!(state.world_id, 42);
        assert_eq!(state.position, [123.5, 0.0, 0.0]);
        assert_eq!(MAPPING_NAME, "Local\\HalfCraft_v1");
    }
    #[test]
    fn rejects_wrong_layout_and_dead_host() {
        let mut words = fixture();
        assert_eq!(snapshot(|at| words[at / 4], 8101), Err(LinkError::Stale));
        assert_eq!(snapshot(|at| words[at / 4], 99), Err(LinkError::Stale));
        words[HEADER_LAYOUT / 4] ^= 1;
        assert_eq!(
            snapshot(|at| words[at / 4], 200),
            Err(LinkError::Incompatible)
        );
    }
    #[test]
    fn odd_sequence_has_bounded_cost() {
        let mut words = fixture();
        words[(HOST_STATE_OFFSET + HOST_STATE_SEQ) / 4] = 1;
        let mut reads = 0;
        assert_eq!(
            snapshot(
                |at| {
                    reads += 1;
                    words[at / 4]
                },
                200
            ),
            Err(LinkError::Busy)
        );
        assert!(reads < 20);
    }
    #[test]
    fn rejects_torn_snapshot() {
        let words = fixture();
        let mut seq_reads = 0;
        assert_eq!(
            snapshot(
                |at| {
                    if at == HOST_STATE_OFFSET + HOST_STATE_SEQ {
                        seq_reads += 1;
                        return seq_reads * 2;
                    }
                    words[at / 4]
                },
                200
            ),
            Err(LinkError::Busy)
        );
    }
    #[test]
    fn rejects_session_change_during_read() {
        let words = fixture();
        let mut sessions = 0;
        assert_eq!(
            snapshot(
                |at| {
                    if at == HEADER_HOST_SESSION {
                        sessions += 1;
                        return sessions;
                    }
                    words[at / 4]
                },
                200
            ),
            Err(LinkError::Stale)
        );
    }
    #[test]
    fn rejects_nan_pose() {
        let mut words = fixture();
        words[(HOST_STATE_OFFSET + HOST_STATE_YAW) / 4] = f32::NAN.to_bits();
        assert_eq!(
            snapshot(|at| words[at / 4], 200),
            Err(LinkError::InvalidState)
        );
    }
}

/// Validate producer/consumer distance before touching ring memory. One native
/// tick handles at most 128 entries; unread data stays queued for the next tick.
fn read_input_batch(
    head: u64,
    tail: u64,
    mut read: impl FnMut(usize) -> u32,
) -> Result<(Vec<input::InputEvent>, u64), LinkError> {
    let pending = head.checked_sub(tail).ok_or(LinkError::InvalidState)?;
    if pending > INPUT_RING_ENTRIES {
        return Err(LinkError::InvalidState);
    }
    let count = pending.min(128);
    let mut events = Vec::with_capacity(count as usize);
    for i in 0..count {
        let index = ((tail + i) & (INPUT_RING_ENTRIES - 1)) as usize;
        let at = INPUT_RING_OFFSET + INPUT_RING_DATA_OFF as usize + index * INPUT_EVENT_SIZE;
        events.push(input::InputEvent::from_words(std::array::from_fn(|word| {
            read(at + word * 4)
        })));
    }
    Ok((events, tail + count))
}

#[cfg(test)]
mod ring_tests {
    use super::*;
    #[test]
    fn malformed_distance_never_reads_memory() {
        for (head, tail) in [(4, 5), (INPUT_RING_ENTRIES + 1, 0)] {
            assert_eq!(
                read_input_batch(head, tail, |_| panic!("invalid ring must not be read")),
                Err(LinkError::InvalidState)
            );
        }
    }
    #[test]
    fn wrap_and_budget_preserve_order() {
        let start = INPUT_RING_ENTRIES - 1;
        let mut reads = Vec::new();
        let (events, tail) = read_input_batch(start + 200, start, |at| {
            reads.push(at);
            0
        })
        .unwrap();
        assert_eq!(events.len(), 128);
        assert_eq!(tail, start + 128);
        assert_eq!(
            reads[0],
            INPUT_RING_OFFSET
                + INPUT_RING_DATA_OFF as usize
                + (INPUT_RING_ENTRIES as usize - 1) * INPUT_EVENT_SIZE
        );
        assert_eq!(reads[4], INPUT_RING_OFFSET + INPUT_RING_DATA_OFF as usize);
    }
}
