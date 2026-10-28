# Block excavation — implementation status

This clipping library is a geometry foundation and is not used to mutate the live BSP.
A separate, finite voxel-replacement implementation is connected to the game: see
[experimental voxel sandbox](voxel-sandbox.md). `source/src/core/hc_excavation.h` subtracts one axis-aligned cell from a triangle,
retaining the surrounding surface and its winding. The same operation can be used by a future
render mesh builder and collision builder. Do not remove Minecraft collision alone: Source would
still render and trace the original wall.

## Implemented and tested

- Convex half-space clipping against the six cell planes, rather than dropping whole triangles
  whose centre happens to fall in a cell (which creates oversized holes).
- Fixed-size polygon scratch storage, no per-plane heap allocation; caller reuses the output vector.
- Bounding-box rejection before clipping, degenerate-fragment removal, coincident-face handling.
- Engine-independent C++17 tests for area, winding, negative coordinates, vertical surfaces,
  empty cuts, complete removal and repeated cuts, including 5,000 deterministic random cuts.

Run on Linux (no game/SDK installation needed):

```sh
g++ -std=c++17 -Wall -Wextra -Werror -fsanitize=address,undefined -g \
  -Isource/src tools/tests/excavation_test.cpp -o /tmp/halfcraft-excavation-test
/tmp/halfcraft-excavation-test
```

## Region state and stone interiors

`hc_excavation_cells.h` now provides per-map removed-cell storage with an explicit cell budget,
region-local surface subtraction and bounded draining of dirty regions. Removing the same cell twice
returns false, so a caller can gate loot on the accepted mutation. Negative coordinates use floor
region division. Neighbours across region boundaries are dirtied and shared tunnel faces are omitted.

The interior builder emits correctly wound unit quads facing the empty cell. Its adapter callback must
confirm the whole neighbouring face was originally solid: unknown space and air must return false.
Partially solid neighbours require brush-level clipping and are not yet supported by this builder.
These are candidate **stone** quads, not actual Source renderables or physics objects. The class is
single-threaded and has no persistence/network protocol yet. `clear()` is a full reset, not an undo;
an engine adapter must also discard meshes when resetting it.

Additional tests cover cross-region neighbour culling, winding on all six faces, dirty upload budgets,
negative coordinates, duplicate removals, capacity limits, map reset and region-local surface cuts.

## Remaining integration (required before enabling)

1. Maintain server-authoritative excavated cell sets keyed by map slot and cell; persist them with
   Half-Life saves and Minecraft rollback, and stream only changed regions. Validate reach, map,
   build bounds and tool on the server; never trust client progress or material selection.
2. Suppress original BSP surfaces in excavated regions on **both supported Source engines** and
   render clipped replacements. This requires engine/renderer work, not an overlay placed on top
   of a wall. Preserve UVs, material IDs, lighting and displacement handling in the mesh adapter;
   this geometry-only helper deliberately does not carry those attributes.
3. Update Source world traces, static physics, streamed Minecraft collision, projectile traces and
   navigation consistently. Dynamic doors, NPCs, props, sky and triggers should not be mined as BSP.
4. Generate newly exposed interior cube faces with a stone material; omit faces between excavated
   neighbours. Original surface subtraction alone cannot produce tunnel walls in a solid brush.
5. Map Source surface properties/materials to Minecraft block states (stone/concrete -> stone,
   brick -> bricks, wood -> planks, dirt -> dirt; unknown -> stone). Material metadata is not in the
   current collision stream; it must be transmitted, not guessed from triangle orientation.
6. Use Minecraft's normal mining progress, tool rules, cracks, particles and loot on the integrated
   server, with one drop per accepted cell removal, including cancellation and creative mode.

For performance, index cells by collision/render region and rebuild only dirty regions plus their
face neighbours, outside the frame-critical path with a bounded per-tick upload budget. Always clip
from the original mesh, not successively from the last fragmented result. Bound excavation depth,
cell count and generated vertices before allocation. No frame-rate improvement is claimed without
measurements in HL2 and HL2DM; the existing rendering and collision paths are unchanged here.
