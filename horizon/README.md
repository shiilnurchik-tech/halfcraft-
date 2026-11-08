# HalfCraft for Horizon — native Rust port, in progress

The target is the full HalfCraft feature set, not a status-only mod. The Fabric
client remains usable during migration. **This native build is not yet playable
HalfCraft** and is not included in the ordinary release ZIP.

Pinned Horizon source: `11102a56510c0a1b546339ab45e1b141c85e8ebc`.

## Native integration implemented

- Windows shared-memory transport using the existing Source protocol/layout.
- Default passive observation without claiming a client slot.
- Explicit `/halfcraft connect`: claim the client PID atomically, publish its
  heartbeat, consume the Source input ring, and release ownership on disconnect,
  world stop or lost session. A nonzero existing PID is **not** evicted. Do not run
  Fabric and Horizon against the same Source process at the same time.
- Bounded ring draining (128 events/tick), signed event decoding, physical SDL
  WASD/jump/sneak state, release-all, one-shot impulses, push-state tracking, and
  per-channel fragmented UTF-8 strings (64 KiB limit; overflow discarded to END).
- Actual Horizon movement-input and player-impulse hooks; hints go to the action
  bar, pickups/captions to chat. Native keyboard movement is retained until Source
  has supplied a key event. Menu/loading/stale-session guards clear held state.
- `/halfcraft follow`: the integrated server moves its single spectator player
  to Source's current position/look. This runs on the authoritative server tick,
  not by editing a client snapshot. It refuses non-Overworld/non-spectator/dead
  players, multiple online players, dedicated servers, stale poses and invalid
  coordinates. It never changes game mode or writes terrain.
- `/halfcraft unfollow`, `/halfcraft disconnect`, `/halfcraft status`.
- Shutdown cleanup, portable decoder tests, real named-mapping/ownership/input
  tests on Windows, schema parity, and compilation against actual upstream APIs.

**Readiness is deliberately withheld:** the MC_IN_WORLD flag remains zero.
Source therefore retains its player physics, HUD and keyboard ownership; it does
not yet forward normal game keys through the full Fabric input path. The new
native input consumer/hooks are wired, but normal Source-window controls cannot
be called end-to-end ready until world/collision export permits that handshake.
Continuous base-push movement, mouse/look, action rebinding and arbitrary Source
commands are not applied yet. Impulses are converted from blocks/second to
blocks/tick exactly once. Source command strings are not executed as console code.

## Test the current position synchronization

Use a **disposable single-player Overworld** in Horizon. Switch it to spectator
mode yourself (`/gamemode spectator`, with cheats enabled). Start Source with the
existing HalfCraft DLLs; in Horizon run:

```text
/halfcraft follow
/halfcraft status
```

Move in Source: the server-tick adapter follows Source's coordinates and look.
Horizon still draws its own world, **not Source BSP geometry**. Stop with
`/halfcraft unfollow` or release the connection with `/halfcraft disconnect`.
A stale occupied PID is intentionally not stolen; close the competing client or
restart Source if its slot was left by a crashed client.

These are developer test instructions, not an in-game verification claim. No
Horizon game session has been run in the development sandbox.

## Build and checks

Requires Horizon's launcher/native-build prerequisites. The manifest pins the
**game source**, not a launcher release number. Native mods run with your account's
privileges; use a separate instance. From the repository root:

```text
horizon-launcher native-build horizon/native --instance HalfCraft --trust-native-code
```

The launcher compiles the game with the mod. This is not a Fabric JAR or a drop-in
prebuilt DLL. No `.hnmod` or tested game executable is supplied yet.

Without fetching or building Horizon:

```sh
cargo test --locked --manifest-path horizon/bridge-tests/Cargo.toml
cargo fmt --manifest-path horizon/bridge-tests/Cargo.toml --check
python tools/build/horizon_protocol.py --check  # Python 3.12+
```

Regenerate offsets/constants after schema edits with
`python tools/build/horizon_protocol.py`. CI stages the adapter in the pinned
Horizon workspace, retaining upstream vendored graphics patches and lockfile.
A standalone `cargo check` of the API crate does not inherit those patches.

## Remaining full-port work

- World/texture/mesh export, incremental streaming and backpressure.
- BSP triangles/sub-voxel collision, smooth movement, water, transitions and
  survival player control; then enable the ordinary gameplay handshake.
- Mouse/buttons/scroll/text, screens, command execution with player permissions,
  health/damage, inventory, gravity gun and save/checkpoint parity.
- Overlay/HUD composition, avatar/entities, textures and connected voxel sandbox.
- In-game reconnect/death/loading/level-change/shutdown tests and performance
  comparisons with the Fabric client.

The existing Fabric/Source gameplay implementations and wire layout are unchanged.
