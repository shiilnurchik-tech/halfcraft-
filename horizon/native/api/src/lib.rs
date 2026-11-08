//! Experimental Horizon adapter; not yet a replacement for the Fabric client.
mod bridge;
use bridge::{HostSnapshot, LinkError, Mapping};
use horizon_client::mod_api::{Hooks, Host, Text, TickEvent};
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

pub fn hooks() -> Box<dyn Hooks> {
    Box::new(HalfCraft {
        state: Mutex::new(State::default()),
    })
}
struct HalfCraft {
    state: Mutex<State>,
}
#[derive(Default)]
struct State {
    mapping: Option<Mapping>,
    next_poll: Option<Instant>,
    status: Option<String>,
}
impl State {
    fn poll(&mut self) -> Result<HostSnapshot, LinkError> {
        if self.mapping.is_none() {
            self.mapping = Some(Mapping::open()?);
        }
        let result = self.mapping.as_ref().unwrap().read();
        if result.is_err() {
            self.mapping = None;
        }
        result
    }
}
impl Hooks for HalfCraft {
    fn tick(&self, host: &mut dyn Host, _event: &mut TickEvent) {
        // Hooks are shared/reentrant; never wait on another dispatch.
        let Ok(mut state) = self.state.try_lock() else {
            return;
        };
        let now = Instant::now();
        if state.next_poll.is_some_and(|next| now < next) {
            return;
        }
        state.next_poll = Some(now + Duration::from_secs(1));
        let message = match state.poll() {
            Ok(snapshot) => format!(
                "HalfCraft: Source connected, world {:08X}, session {} (observer only)",
                snapshot.world_id, snapshot.session
            ),
            Err(error) => format!("HalfCraft: Source {error:?} (observer only)"),
        };
        // Don't steal the action bar every frame or publish client-ready flags.
        // No host calls while holding the reentrancy guard.
        let changed = state.status.as_ref() != Some(&message);
        state.status = Some(message.clone());
        drop(state);
        if changed {
            eprintln!("{message}");
            if host.client_state().is_some_and(|client| client.logged_in) {
                host.client_action_bar(Text::literal(message), false);
            }
        }
    }
}
