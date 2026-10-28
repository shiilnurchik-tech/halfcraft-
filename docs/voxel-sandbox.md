# Experimental mineable voxel sandbox

This adds an opt-in **finite sandbox**, not destructive editing of the original BSP. A 25 x 17 x 25
volume around the player is copied from streamed collision into real Minecraft stone blocks.
Minecraft handles mining time, pickaxes, cracks, particles, Silk Touch/Fortune rules and cobblestone
loot. The existing world mesh/entity exporter sends these blocks, cracks and items to Source.
All generated terrain currently uses **stone**, not the original Source surface materials.

## Build and start

Rebuild **both** the Minecraft mod and Source DLLs: the shared protocol now has a new layout hash.
Use the existing Windows build prerequisites from README (JDK 25, VS/SDK, installed game):

```text
make build
make mc
```

Use a disposable/test mirror world (`make test-start` and `make mc-test`, see README). Once both games
are linked and the surrounding collision/chunks have loaded, stand still in an open space and enter
these commands in the **Half-Life console**, in this order:

```text
sv_cheats 1
hc_voxel_sandbox 1
hc_mc halfcraft_voxelize
```

Wait for `Voxel sandbox ready` in Minecraft chat. If nearby collision/chunks are not loaded, the
command rejects the entire conversion without changing anything; wait and retry. Conversion applies
at most 512 block edits / 2 ms per server tick (one update cannot be interrupted). Moving while it builds cancels it. A full bedrock enclosure
bounds the copied area. Existing builds/containers/fluids are not overwritten. Cells intersecting
the player are left open. Use the diamond pickaxe from the starter kit to dig the replacement stone.
Place torches where needed: the original map's lighting still applies.

To stop, **first** run:

```text
hc_mc halfcraft_voxelize stop
```

This returns the player to their starting point and restores the original states of the generated
cells incrementally with the same per-tick budget. A new conversion is rejected until restoration finishes. Then optionally run `hc_voxel_sandbox 0`. Rendering settings are restored automatically when
the ready flag clears, on link loss, level initialization and client shutdown. Do not turn off the
Source cvar while still underground in the sandbox.

## What is connected

- `HostCommand` dispatches the conversion on the integrated server; `HalfCraft` registers tick and
  shutdown cleanup. Conversion rejects unknown regions, unloaded chunks and invalid build heights.
- `HostCollision` bypasses the old host triangles/voxels for the copied volume after it becomes ready;
  block queries, player movement and crosshair targeting use actual Minecraft blocks instead.
- A new `McFlags` bit reports readiness through the existing shared-memory state. `hc_client.cpp`
  hides BSP and static props only when this flag AND the opt-in Source cvar are set.
- Source's original PVS/occlusion are disabled in this mode so they cannot hide replacement blocks
  behind the old walls. Previous render/culling cvar values are retained and restored.
- The movement command carries a sandbox bit. A replicated, cheat-protected server cvar gates its
  special jump handling. Original Source ground/ladder attachment is skipped for these commands.
- Collision epoch changes cancel the conversion and request restoration on the server thread.

## Important limitations

- **Not campaign compatible.** Original Source BSP traces, VPhysics, NPC navigation and weapon rays
  still exist. Source NPCs, doors, props, original water and scripted events are not converted. Do not
  use Source weapons, ladders, vehicles or level exits to evaluate digging; test with Minecraft tools.
- All copied terrain is stone. Slopes/thin shapes become full blocks at a 5% voxel fill threshold.
  There is no texture/material classification yet. The screenshot's original facade is not retained.
- This is a transient sandbox. Stop restores generated positions even if you mined/replaced them;
  collected drops are not revoked. Builds in initially empty, unconverted cells are not cleaned up.
- **Do not save/load a campaign in this mode.** Crash recovery of temporary terrain is not implemented;
  a crash or autosave can persist generated blocks. Use a disposable mirror world and backup saves.
- Disabling PVS/occlusion costs FPS. Finite geometry and incremental edits bound conversion work,
  but no overall performance improvement is claimed. Runtime frame/upload measurements are pending.

## Validation

The C++ excavation tests and protocol generator consistency check run in the Linux workspace. The
new Java/Source integration has **not** been compiled or run here: there is no JDK 25, Windows Source
SDK build environment or installed HL2, and Fabric/Mojang dependency hosts are not accessible here.
Required in-game acceptance checks on **both HL2 and HL2DM**:

1. Start with mode off: original campaign movement/rendering is unchanged.
2. Run conversion before collision loads: refusal, no terrain mutation or hidden world.
3. Convert in an open area: no block intersects player; bedrock boundary is intact.
4. Mine stone with a pickaxe: visible cracks/particles, one normal loot result, permanent hole during
   this session, newly exposed stone neighbours, items can be picked up.
5. Walk into the excavated cell: no original host triangle pushes the player out; jump and sneak work.
6. Stop underground: return to start; original render/culling values restored and generated cells gone.
7. Stop during build or move while building: partial edits restored.
8. Load another map, disconnect, close the server: readiness clears and view is restored.
9. Repeat start/stop: no duplicates, no additional readiness handshake stuck across map loads.
10. Test pre-existing chest/build, nearby boundary mining and chunk streaming for lost contents or leaks.
