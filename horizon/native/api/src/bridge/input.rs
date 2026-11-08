//! Source input decoding and tick state, shared by native hooks and regression tests.
use super::generated::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputEvent {
    pub kind: u16,
    pub code: u16,
    pub values: [i32; 3],
}
impl InputEvent {
    pub fn from_words(words: [u32; 4]) -> Self {
        Self {
            kind: words[0] as u16,
            code: (words[0] >> 16) as u16,
            values: [words[1] as i32, words[2] as i32, words[3] as i32],
        }
    }
}

#[derive(Default)]
pub struct Inputs {
    keys: BTreeSet<u16>,
    remote_keys: bool,
    strings: BTreeMap<u16, Vec<u8>>,
    // Once oversized, discard through END, not just the current fragment.
    dropping: BTreeSet<u16>,
    base: [f64; 3],
    last_push: u64,
    pushing: bool,
    released: [f64; 3],
    impulse: [f64; 3],
}
impl Inputs {
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    pub fn has_remote_keys(&self) -> bool {
        self.remote_keys
    }
    pub fn movement(&self) -> (f32, f32, bool, bool) {
        let down = |key| i32::from(self.keys.contains(&key));
        // SDL physical scancodes; action rebinding is a separate engine adapter.
        (
            (down(26) - down(22)) as f32,
            (down(4) - down(7)) as f32,
            self.keys.contains(&44),
            self.keys.contains(&225) || self.keys.contains(&229),
        )
    }
    pub fn accept(&mut self, event: InputEvent, now: u64) -> Option<(u16, String)> {
        let [a, b, c] = event.values;
        match u32::from(event.kind) {
            IN_KEY => {
                self.remote_keys = true;
                if a != 0 {
                    self.keys.insert(event.code);
                } else {
                    self.keys.remove(&event.code);
                }
            }
            IN_RELEASE_ALL => self.clear(),
            IN_PUSH => {
                // A delayed stop must not revive a push that already went stale.
                self.expire(now);
                let pushing = a != 0 || b != 0 || c != 0;
                if self.pushing && !pushing {
                    self.released = self.base;
                }
                self.base = [a, b, c].map(|v| f64::from(v) / 1000.0);
                self.pushing = pushing;
                self.last_push = now;
                if pushing {
                    self.released = [0.0; 3];
                }
            }
            IN_IMPULSE => {
                for (total, value) in self.impulse.iter_mut().zip([a, b, c]) {
                    *total += f64::from(value) / 1000.0;
                }
            }
            IN_STRING => return self.fragment(event.code, event.values),
            _ => {}
        }
        None
    }
    fn fragment(&mut self, code: u16, words: [i32; 3]) -> Option<(u16, String)> {
        let channel = code & STRING_CHANNEL_MASK as u16;
        let length = ((u64::from(code) >> STRING_BYTES_SHIFT) & 15) as usize;
        let end = u64::from(code) & STRING_END != 0;
        if length > STRING_PIECE_BYTES as usize {
            self.dropping.insert(channel);
        }
        if !self.dropping.contains(&channel) {
            let pending = self.strings.entry(channel).or_default();
            let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
            if pending.len() + length > 65536 {
                self.strings.remove(&channel);
                self.dropping.insert(channel);
            } else {
                pending.extend_from_slice(&bytes[..length]);
            }
        }
        if !end {
            return None;
        }
        if self.dropping.remove(&channel) {
            self.strings.remove(&channel);
            return None;
        }
        let bytes = self.strings.remove(&channel).unwrap_or_default();
        String::from_utf8(bytes).ok().map(|text| (channel, text))
    }
    fn expire(&mut self, now: u64) {
        if self.pushing && (now < self.last_push || now - self.last_push > PUSH_STALE_MS) {
            self.pushing = false;
            self.base = [0.0; 3];
            self.released = [0.0; 3];
        }
    }
    /// Drain only discrete impulses. Until the engine adapter can apply base
    /// velocity without accumulating acceleration, it must not transfer a base
    /// push's release into momentum either.
    pub fn take_impulse(&mut self) -> [f32; 3] {
        let impulse = self.impulse.map(|v| (v / 20.0) as f32);
        self.impulse = [0.0; 3];
        impulse
    }
    /// Combined impulse/release transfer for a future base-velocity adapter;
    /// ongoing base movement is read separately with base_velocity().
    pub fn take_push(&mut self, now: u64) -> [f32; 3] {
        self.expire(now);
        let result = std::array::from_fn(|i| ((self.impulse[i] + self.released[i]) / 20.0) as f32);
        self.impulse = [0.0; 3];
        self.released = [0.0; 3];
        result
    }
    pub fn base_velocity(&mut self, now: u64) -> Option<[f64; 3]> {
        self.expire(now);
        self.pushing.then_some(self.base)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(kind: u32, code: u16, values: [i32; 3]) -> InputEvent {
        InputEvent {
            kind: kind as u16,
            code,
            values,
        }
    }
    #[test]
    fn keys_release_and_opposites() {
        let mut inputs = Inputs::default();
        inputs.accept(event(IN_KEY, 26, [1, 0, 0]), 0);
        assert_eq!(inputs.movement().0, 1.0);
        inputs.accept(event(IN_KEY, 22, [1, 0, 0]), 0);
        assert_eq!(inputs.movement().0, 0.0);
        inputs.accept(event(IN_RELEASE_ALL, 0, [0; 3]), 0);
        assert_eq!(inputs.movement(), (0.0, 0.0, false, false));
    }
    #[test]
    fn impulses_sum_and_drain_once() {
        let mut inputs = Inputs::default();
        for _ in 0..2 {
            inputs.accept(event(IN_IMPULSE, 0, [1000, 2000, -1000]), 0);
        }
        assert_eq!(inputs.take_push(0), [0.1, 0.2, -0.1]);
        assert_eq!(inputs.take_push(0), [0.0; 3]);
    }
    #[test]
    fn base_push_explicit_stop_vs_timeout() {
        let mut inputs = Inputs::default();
        inputs.accept(event(IN_PUSH, 0, [2000, 0, 0]), 0);
        assert_eq!(inputs.base_velocity(200), Some([2.0, 0.0, 0.0]));
        inputs.accept(event(IN_PUSH, 0, [0; 3]), 200);
        assert_eq!(inputs.take_push(200), [0.1, 0.0, 0.0]);
        inputs.accept(event(IN_PUSH, 0, [2000, 0, 0]), 300);
        assert_eq!(inputs.base_velocity(801), None);
        inputs.accept(event(IN_PUSH, 0, [0; 3]), 802);
        assert_eq!(inputs.take_push(802), [0.0; 3]);
    }
    #[test]
    fn impulse_only_adapter_does_not_apply_unimplemented_base_momentum() {
        let mut inputs = Inputs::default();
        inputs.accept(event(IN_PUSH, 0, [2000, 0, 0]), 0);
        inputs.accept(event(IN_PUSH, 0, [0; 3]), 200);
        assert_eq!(inputs.take_impulse(), [0.0; 3]);
        inputs.accept(event(IN_IMPULSE, 0, [1000, 0, 0]), 200);
        assert_eq!(inputs.take_impulse(), [0.05, 0.0, 0.0]);
        assert_eq!(inputs.take_impulse(), [0.0; 3]);
    }
    #[test]
    fn strings_unicode_across_fragments_and_channels() {
        let mut inputs = Inputs::default();
        assert!(
            inputs
                .accept(
                    event(
                        IN_STRING,
                        1 | (2 << 8),
                        [i32::from_le_bytes([0xd0, 0x9f, 0, 0]), 0, 0]
                    ),
                    0
                )
                .is_none()
        );
        let end = STRING_END as u16;
        assert_eq!(
            inputs.accept(event(IN_STRING, 2 | (1 << 8) | end, [65, 0, 0]), 0),
            Some((2, "A".into()))
        );
        assert_eq!(
            inputs.accept(event(IN_STRING, 1 | end, [0; 3]), 0),
            Some((1, "П".into()))
        );
    }
    #[test]
    fn oversize_discards_to_terminator_then_recovers() {
        let mut inputs = Inputs::default();
        for _ in 0..5500 {
            assert!(
                inputs
                    .accept(event(IN_STRING, 1 | (12 << 8), [0; 3]), 0)
                    .is_none()
            );
        }
        assert!(
            inputs
                .accept(
                    event(IN_STRING, 1 | (1 << 8) | STRING_END as u16, [65, 0, 0]),
                    0
                )
                .is_none()
        );
        assert_eq!(
            inputs.accept(
                event(IN_STRING, 1 | (1 << 8) | STRING_END as u16, [65, 0, 0]),
                0
            ),
            Some((1, "A".into()))
        );
    }
    #[test]
    fn wire_event_decodes_signed_values() {
        assert_eq!(
            InputEvent::from_words([IN_KEY | (225 << 16), u32::MAX, 2, 3]),
            event(IN_KEY, 225, [-1, 2, 3])
        );
    }
}
