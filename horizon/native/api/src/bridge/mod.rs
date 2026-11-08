//! Read-only Source transport. Deliberately never advertises Minecraft readiness:
//! until rendering/collision are ported, Source must retain player authority.
mod generated;
#[cfg(windows)]
mod windows;
use generated::*;

pub const VIEW_BYTES: usize = HOST_STATE_OFFSET + HOST_STATE_SIZE;
const HOST_TIMEOUT_MS: u64 = 8_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkError {
    UnsupportedPlatform,
    Unavailable,
    Incompatible,
    Stale,
    Busy,
    InvalidState,
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
