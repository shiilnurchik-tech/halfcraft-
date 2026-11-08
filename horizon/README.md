# HalfCraft for Horizon — experimental, incomplete

This is the beginning of a **native Rust port**, alongside (not replacing) the
Fabric client. It is **not yet playable HalfCraft** and is not included in the
normal HalfCraft release ZIP. No `.hnmod` or tested Horizon executable is supplied.

## Implemented

- Native trait-mod entry point using Horizon's `Hooks::tick`, `Host::client_state`
  and action-bar service, pinned to source revision
  `11102a56510c0a1b546339ab45e1b141c85e8ebc`.
- Windows read-only connection to the existing Source mapping
  `Local\HalfCraft_v1`; Source must already be running with HalfCraft installed.
- Schema-generated offsets/layout fingerprint; host heartbeat, session, layout,
  bounded seqlock reads and finite-position validation.
- Connection/world/session status in Horizon's log and, on status changes while
  logged in, its action bar. Unavailable/incompatible hosts are reported, not
  silently accepted. Retry is capped at once per second.
- Portable regression tests using the **same bridge source** as the native mod,
  plus Windows tests and a pinned upstream API compile check in CI.

The mapping is opened **read-only**: no client PID/heartbeat, ready flags, ring
indices or world buffers are changed. Source keeps its ordinary player authority.
Do not treat a “Source connected” status as confirmation of a playable port.
The transport is Windows-only because the current Source integration is Win32;
other platforms return `UnsupportedPlatform` without creating a fake link.

## Developer build

Requires Horizon's launcher/native-build prerequisites (including its Rust/native
build tools). A Fabric JAR cannot be used here. Launcher version numbers do not
identify the game's native API; the manifest pins the game's source revision.
From the repository root:

```text
horizon-launcher native-build horizon/native --instance HalfCraft --trust-native-code
```

Native mods execute with your account's privileges. Use a separate test instance.
The launcher builds the game with the mod; this is **not a prebuilt DLL** to drop
into a mods directory. The command and actual in-game integration have not yet
been verified in this repository's development environment.

For bridge tests without fetching/building Horizon:

```sh
cargo test --locked --manifest-path horizon/bridge-tests/Cargo.toml
cargo fmt --manifest-path horizon/bridge-tests/Cargo.toml --check
python tools/build/horizon_protocol.py --check  # Python 3.12+
```

After schema changes, regenerate with `python tools/build/horizon_protocol.py`.

## Required before playable release

1. Validate the native-mod build against the pinned upstream API and run in game.
2. Port world/texture/mesh export and incremental streaming/backpressure.
3. Port Source collision queries, player/camera synchronization and teleport/session
   resets; only then enable the normal client-ready handshake.
4. Port input, inventory, damage, gravity-gun block interactions and saves.
5. Port overlay/HUD, avatar/entities and the connected voxel sandbox.
6. Exercise reconnect, death/loading screens, map changes and both game processes
   shutting down. Compare gameplay and performance with the Fabric implementation.

Those features remain implemented in `minecraft/` only. Existing Fabric/Source
runtime code and the shared-memory layout are unchanged by this experiment.
