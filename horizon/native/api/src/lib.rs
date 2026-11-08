//! Native HalfCraft integration. Source remains authoritative until mesh export
//! and collision are ready; the client never advertises MC_IN_WORLD prematurely.
mod bridge;
use bridge::{HostSnapshot, LinkError, Mapping, generated::*, input::Inputs};
use horizon_client::mod_api::{
    ClientTickPostEvent, CommandEvent, CommandsEvent, Flow, Hooks, Host, MovementInputEvent,
    NativeAuthority, NativeScreenKind, ServerStoppingEvent, ServerTickEvent, ServerTickPhase, Text,
    TickEvent,
};
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
    enabled: bool,
    follow: bool,
    inputs: Inputs,
    latest: Option<HostSnapshot>,
    received: Option<Instant>,
    playable: bool,
    next_poll: Option<Instant>,
    status: Option<String>,
    follow_warning: Option<&'static str>,
}
impl State {
    fn reset(&mut self) {
        self.mapping = None;
        self.inputs.clear();
        self.latest = None;
        self.received = None;
        self.playable = false;
    }
    fn fresh(&self) -> bool {
        self.received
            .is_some_and(|when| when.elapsed() < Duration::from_millis(250))
    }
    fn poll(&mut self) -> Result<HostSnapshot, LinkError> {
        if self.mapping.is_none() {
            self.mapping = Some(if self.enabled {
                Mapping::connect()?
            } else {
                Mapping::open()?
            });
        }
        self.mapping.as_ref().unwrap().read()
    }
    fn active_source(snapshot: HostSnapshot) -> bool {
        snapshot.flags & HOST_IN_GAME != 0 && snapshot.flags & (HOST_LOADING | HOST_MENU_OPEN) == 0
    }
}
impl Hooks for HalfCraft {
    fn server_stopping(&self, _host: &mut dyn Host, _event: &mut ServerStoppingEvent) {
        // Lifecycle teardown must release ownership even if a tick is finishing.
        // No Host methods are called while this mutex is held.
        if let Ok(mut state) = self.state.lock() {
            state.reset();
            state.enabled = false;
            state.follow = false;
        }
    }
    fn commands(&self, _host: &mut dyn Host, event: &mut CommandsEvent<'_>) {
        event.commands.push("halfcraft");
    }
    fn command(&self, host: &mut dyn Host, event: &mut CommandEvent<'_>) -> Flow {
        let mut words = event.command.split_whitespace();
        if words.next() != Some("halfcraft") {
            return Flow::Continue;
        }
        let command = words.next().unwrap_or("help");
        let trailing = words.next();
        let message = {
            let Ok(mut state) = self.state.try_lock() else {
                host.chat("HalfCraft is busy; retry the command.");
                return Flow::Cancel;
            };
            match (command,trailing) {
                ("connect",None) | ("follow",None) => {
                    if !state.enabled {
                        state.reset();
                        match Mapping::connect() {
                            Ok(mapping) => { state.mapping=Some(mapping); state.enabled=true; }
                            Err(error) => {
                                drop(state);
                                host.chat(&format!("HalfCraft: cannot connect to Source: {error:?}"));
                                return Flow::Cancel;
                            }
                        }
                    }
                    state.next_poll=None;
                    if command=="follow" { state.follow=true; state.follow_warning=None; }
                    "HalfCraft connected. Follow requires a single-player Overworld in spectator mode. Source still owns physics; rendering/collision export is not ready.".to_owned()
                }
                ("disconnect",None) => {
                    state.reset(); state.enabled=false; state.follow=false;
                    "HalfCraft disconnected; input ownership released.".to_owned()
                }
                ("unfollow",None) => { state.follow=false; "HalfCraft Source-follow stopped.".to_owned() }
                ("status",None) => format!("HalfCraft: input={}, follow={}, fresh={}; native world export/collision not ready.", state.enabled,state.follow,state.fresh()),
                _ => "HalfCraft: /halfcraft connect | disconnect | follow | unfollow | status. Use a disposable test world; no full gameplay handshake yet.".to_owned(),
            }
        };
        host.chat(&message);
        Flow::Cancel
    }
    fn tick(&self, host: &mut dyn Host, _event: &mut TickEvent) {
        let Ok(mut state) = self.state.try_lock() else {
            return;
        };
        if state.enabled {
            return;
        }
        let now = Instant::now();
        if state.next_poll.is_some_and(|next| now < next) {
            return;
        }
        state.next_poll = Some(now + Duration::from_secs(1));
        let message = match state.poll() {
            Ok(snapshot) => format!(
                "HalfCraft: Source world {:08X}, session {}. /halfcraft connect",
                snapshot.world_id, snapshot.session
            ),
            Err(error) => {
                state.reset();
                format!("HalfCraft: Source {error:?}")
            }
        };
        let changed = state.status.as_ref() != Some(&message);
        state.status = Some(message.clone());
        drop(state);
        if changed {
            eprintln!("{message}");
        }
        // Passive observation never claims input or floods the HUD.
        let _ = host;
    }
    fn client_tick_post(&self, host: &mut dyn Host, _event: &mut ClientTickPostEvent) {
        let client = host.client_state();
        let logged_in = client.as_ref().is_some_and(|client| client.logged_in);
        let playing = client
            .as_ref()
            .is_some_and(|client| client.screen == NativeScreenKind::Playing);
        let Ok(mut state) = self.state.try_lock() else {
            return;
        };
        if !state.enabled {
            return;
        }
        if !logged_in {
            state.reset();
            state.enabled = false;
            state.follow = false;
            return;
        }
        let now = Instant::now();
        if state.next_poll.is_some_and(|next| now < next) {
            return;
        }
        state.next_poll = None;
        let result = state.poll().and_then(|snapshot| {
            state
                .mapping
                .as_mut()
                .unwrap()
                .drain_input()
                .map(|events| (snapshot, events))
        });
        let (snapshot, events) = match result {
            Ok(pair) => pair,
            Err(error) => {
                state.reset();
                state.next_poll = Some(now + Duration::from_secs(1));
                let message = format!("HalfCraft Source connection lost: {error:?}");
                let changed = state.status.as_ref() != Some(&message);
                state.status = Some(message.clone());
                drop(state);
                if changed {
                    host.chat(&message);
                }
                return;
            }
        };
        if state.latest.is_some_and(|old| {
            old.session != snapshot.session
                || old.collision_epoch != snapshot.collision_epoch
                || old.world_id != snapshot.world_id
        }) {
            state.inputs.clear();
            state.follow_warning = None;
        }
        state.latest = Some(snapshot);
        state.received = Some(now);
        state.playable = playing && State::active_source(snapshot);
        let mut strings = Vec::new();
        if state.playable {
            for event in events {
                if let Some(text) = state.inputs.accept(event, Mapping::now_ms()) {
                    strings.push(text);
                }
            }
        } else {
            state.inputs.clear();
        }
        let impulse = state.inputs.take_push(Mapping::now_ms());
        drop(state);
        if impulse.iter().any(|v| *v != 0.0) {
            host.push_player(impulse);
        }
        for (channel, text) in strings {
            match u32::from(channel) {
                STR_HINT => host.client_action_bar(Text::literal(text), false),
                STR_CAPTION | STR_PICKUP => host.chat(&text),
                // Never turn an untrusted/partial command fragment into a console command.
                STR_COMMAND => host.chat(
                    "HalfCraft: Source command transport received; execution adapter not enabled.",
                ),
                _ => {}
            }
        }
    }
    fn movement_input(&self, _host: &dyn Host, event: &mut MovementInputEvent) {
        let Ok(state) = self.state.try_lock() else {
            return;
        };
        if !state.enabled
            || !state.playable
            || !state.fresh()
            || state.follow
            || !state.inputs.has_remote_keys()
        {
            return;
        }
        let (forward, strafe, jump, sneak) = state.inputs.movement();
        event.forward = forward;
        event.strafe = strafe;
        event.jump = jump;
        event.sneak = sneak;
    }
    fn server_tick(&self, host: &mut dyn Host, event: &mut ServerTickEvent<'_>) {
        if event.context.authority != NativeAuthority::IntegratedServer
            || !matches!(
                event.phase,
                ServerTickPhase::ServerTickStart {
                    world_running: true
                }
            )
        {
            return;
        }
        let snapshot = {
            let Ok(state) = self.state.try_lock() else {
                return;
            };
            if !state.enabled || !state.follow || !state.fresh() {
                return;
            }
            state.latest.filter(|s| State::active_source(*s))
        };
        let Some(snapshot) = snapshot else {
            return;
        };
        let players = host.server_players();
        // Never target an arbitrary LAN guest or a dedicated server's players.
        if players.len() != 1 {
            return;
        }
        let player = &players[0];
        if player.dimension != "minecraft:overworld"
            || player.game_mode != 3
            || player.health <= 0.0
        {
            let warn = {
                let Ok(mut state) = self.state.try_lock() else {
                    return;
                };
                let warn = state.follow_warning.is_none();
                state.follow_warning = Some("spectator");
                warn
            };
            if warn {
                host.action_bar(
                    player.uuid,
                    Text::literal(
                        "HalfCraft follow: use spectator mode in a disposable Overworld.",
                    ),
                );
            }
            return;
        }
        let [x, y, z] = snapshot.position;
        if x.abs() > 29_999_984.0 || z.abs() > 29_999_984.0 || y.abs() > 2048.0 {
            return;
        }
        if player
            .position
            .iter()
            .zip(snapshot.position)
            .all(|(a, b)| (a - b).abs() < 0.0001)
            && (player.yaw - snapshot.yaw).abs() < 0.001
            && (player.pitch - snapshot.pitch).abs() < 0.001
        {
            return;
        }
        let hex = format!("{:032x}", player.uuid);
        let uuid = format!(
            "{}-{}-{}-{}-{}",
            &hex[..8],
            &hex[8..12],
            &hex[12..16],
            &hex[16..20],
            &hex[20..]
        );
        // All fields are finite numeric protocol values, not Source-provided text.
        host.run_command(&format!(
            "teleport {uuid} {x:.8} {y:.8} {z:.8} {:.6} {:.6}",
            snapshot.yaw,
            snapshot.pitch.clamp(-90.0, 90.0)
        ));
    }
}
