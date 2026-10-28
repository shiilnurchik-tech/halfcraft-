# Teleport and level-transition scheduling

Changes target repeated scripted teleports (Kleiner's lab / A Red Letter Day) and level transitions.
They do not make Source map loading asynchronous or remove its native loading screen.

## Changes

- **Nonblocking Minecraft world export.** Section geometry, lights and solids use a single write
  attempt and retain the existing dirty-section retry path. Previously each message could sleep up
  to 500 x 2 ms while Source was loading and not draining its render ring. Clear/atlas now retry in
  order on later frames as well. An unchanged texture atlas is reused across resend requests.
- **HUD first.** Overlay capture/shipping happens before world export. One failed seqlock snapshot
  read no longer hides the HUD or drops puppet ownership for the same live Minecraft process;
  Source keeps the last complete snapshot, not a partially copied one. Actual heartbeat loss,
  death, camera cutscenes and native loading screens retain their existing visibility rules.
- **Superseded teleports.** Source recognizes a new scripted destination while the previous one is
  still awaiting acknowledgment, instead of waiting for its one-second mismatch watchdog. Pending integrated-server teleport tasks check a latest-request token
  before invoking the expensive teleport/chunk-loading operation. Already-running teleports cannot
  be interrupted. Releasing the player waits for the matching server task to complete, and each
  destination gets its own collision-wait timer.
- **Bounded collision work.** All producer paths (including moving entities and explicit requests)
  share the limit of 3 gathers / 2.5 ms per frame. One expensive gather cannot be interrupted, but
  it prevents starting the next one. At most 24 jobs wait for voxelization. Obsolete distant queued
  work is pruned and forgotten so it can be requested on return. Explicit projectile column jobs
  and epoch-clear messages are retained; clear stays ahead of region data and other jobs are
  ordered around the new player location. Waiting sends stop retrying when their epoch changes.
- **Smaller linked-world footprint.** Default Minecraft render distance is 5 chunks, simulation
  distance 4, instead of 8/8. This affects Minecraft builds/entities, not HL2's world detail. For
  distant farms or builds, pass `-Dhalfcraft.renderDistance=8 -Dhalfcraft.simulationDistance=8` to
  Minecraft's JVM. Values are clamped to 2..16. Linked options are saved as before.

## Validation and limits

Offline tests (`tools/tests/stream_budget_test.cpp`, `tools/tests/excavation_test.cpp`) passed with
AddressSanitizer and UBSan. Protocol consistency and `git diff --check` passed. Java integration and
Source DLLs cannot be built/run in this workspace (no Java, Windows toolchain or installed games).
No measured FPS/loading-time gain or confirmation of the reported freeze being fixed is claimed.

After rebuilding BOTH sides, compare the same save before/after:

1. `d1_trainstation_05`: the lab teleporter sequence, including every intermediate destination and
   the final arrival. Check HUD, position, no falls through missing collision, no old teleport replay.
2. A normal `changelevel` and a save load, then a teleport back to an earlier region. Check collision
   was not lost permanently when queued jobs were pruned.
3. Busy moving doors/lifts plus an off-player projectile: priority work must not exceed the budget;
   requested ground must eventually arrive.
4. Capture `hc_perf` on the Source side and F3 timing plus `logs/latest.log` on Minecraft.
   Compare longest stalls, teleport hold duration and render-ring backlog, not just average FPS.
5. Pause/alt-tab and disconnect/reconnect during loading: HUD/input must not remain stuck and a new
   process must not reuse the old process's seqlock snapshot.

A remaining long stall needs logs/profile data: synchronous chunk generation, rollback/save IO,
GPU driver waits and individual SDK geometry gathers cannot be diagnosed from the scene name alone.

## Follow-up audit

The backpressure changes were audited for correctness as well as frame cost:

- A section now retains **one** pending geometry/light/solid batch. Only its unsent tail is retried;
  the shared mesh buffers are not reused until the batch completes. A new resend drops the old batch
  before sending clear/atlas. This avoids both repeated meshing and starvation of lights/solids behind
  repeatedly resent geometry. Dirty updates made while waiting remain queued for a later remesh.
- Avatar texture uploads and empty mesh messages are also nonblocking. One pending texture retains
  its decoded pixels instead of rereading/reallocating them every retry. Oversized textures are rejected
  before allocation so they cannot pin that slot forever.
- Animated sprites reuse a per-animation pixel buffer. Resending the base atlas invalidates animation
  frame markers so cached `shown` values cannot suppress the first update to a newly cleared receiver.
- Collision messages no longer disappear after a two-second full-ring timeout while their regions
  remain marked as harvested. The bounded worker waits until delivery, shutdown or epoch cancellation.
  Queued snapshots of the same region are coalesced. Landing regions are gathered before moving-door
  and projectile work; the worker serves a waiting explicit request after at most three ordinary jobs
  so nearest-first ordering cannot starve distant projectile ground forever.
- Queued teleports are invalidated on link loss/player removal. Requests without an integrated server
  are left pending instead of acknowledging a teleport that was never scheduled.
- Sandbox culling/render cvars are written on mode transitions, not on every frame. Sandbox preflight
  checks loading per region/chunk; pathfinding membership checks no longer allocate BlockPos objects.
  Conversion AND normal restoration now have 512-edit / 2 ms tick budgets (one block update cannot be
  preempted). Shutdown restoration still completes synchronously before save/rollback teardown.

Validation in the Linux workspace: 175 Java files parsed successfully using tree-sitter-java (syntax
only, **not** type/API checking); C++ budget/fairness and excavation tests passed with ASan/UBSan.
Java compilation and in-game acceptance remain pending on the Windows/JDK 25 game environment.
