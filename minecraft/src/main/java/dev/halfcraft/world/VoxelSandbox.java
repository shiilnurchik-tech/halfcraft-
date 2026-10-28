package dev.halfcraft.world;

import dev.halfcraft.mobs.HostNav;
import java.util.ArrayList;
import java.util.List;
import net.fabricmc.fabric.api.event.lifecycle.v1.ServerTickEvents;
import net.fabricmc.fabric.api.event.lifecycle.v1.ServerLifecycleEvents;
import net.minecraft.core.BlockPos;
import net.minecraft.network.chat.Component;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.AABB;

/** Opt-in, finite conversion of streamed Source collision into ordinary mineable Minecraft blocks. */
public final class VoxelSandbox {
	private static final int RADIUS = 12;
	private static final int BELOW = 8;
	private static final int ABOVE = 8;
	private static final int BUDGET = 512;
	private static final long TICK_BUDGET_NS = 2_000_000L;
	private static Job job; // integrated server thread only
	private static volatile AABB active;
	private static volatile boolean cancel;

	private record Edit(BlockPos pos, BlockState before, BlockState after) {
	}

	private static final class Job {
		final ServerLevel level;
		final ServerPlayer player;
		final AABB bounds;
		final List<Edit> edits;
		final net.minecraft.world.phys.Vec3 start;
		final int epoch;
		int cursor;
		boolean ready;
		boolean restoring;

		Job(ServerPlayer player, AABB bounds, List<Edit> edits, int epoch) {
			this.player = player;
			this.level = player.level();
			this.bounds = bounds;
			this.edits = edits;
			this.start = player.position();
			this.epoch = epoch;
		}
	}

	private VoxelSandbox() {
	}

	public static void init() {
		ServerTickEvents.END_SERVER_TICK.register(VoxelSandbox::tick);
		ServerLifecycleEvents.SERVER_STOPPING.register(server -> {
			restore();
			if (job != null) restoreBatch(job, true);
		});
	}

	public static boolean active() {
		return active != null;
	}

	public static boolean contains(BlockPos pos) {
		return contains(pos.getX(), pos.getY(), pos.getZ());
	}

	public static boolean contains(int x, int y, int z) {
		AABB box = active;
		return box != null && x >= box.minX && x < box.maxX && y >= box.minY && y < box.maxY && z >= box.minZ && z < box.maxZ;
	}

	public static boolean overlaps(AABB query) {
		AABB box = active;
		return box != null && box.intersects(query);
	}

	/** Collision consumer thread: the server restores transient edits on its next tick. */
	public static void collisionReset() {
		active = null;
		cancel = true;
	}

	/** Recognized only via hc_mc, which already runs on the integrated server as its owner. */
	public static boolean command(ServerPlayer player, String command) {
		if (command.equals("halfcraft_voxelize stop")) {
			if (job != null && job.ready && !job.restoring && player.level() == job.level) {
				player.teleportTo(job.start.x, job.start.y, job.start.z);
			}
			restore();
			player.sendSystemMessage(Component.literal("Voxel sandbox stopping; original blocks are being restored."));
			return true;
		}
		if (!command.equals("halfcraft_voxelize")) {
			return false;
		}
		if (job != null || !HostNav.inMirror(player.level()) || !HostCollision.active()) {
			player.sendSystemMessage(Component.literal("Voxel sandbox: already running, or Source collision is not ready."));
			return true;
		}
		int epoch = HostCollision.epoch();
		BlockPos origin = player.blockPosition();
		int x0 = origin.getX() - RADIUS, x1 = origin.getX() + RADIUS;
		int y0 = origin.getY() - BELOW, y1 = origin.getY() + ABOVE;
		int z0 = origin.getZ() - RADIUS, z1 = origin.getZ() + RADIUS;
		if (y0 < player.level().getMinY() || y1 >= player.level().getMaxY()) {
			player.sendSystemMessage(Component.literal("Voxel sandbox: too close to the build height limit."));
			return true;
		}
		// Loading is a region/chunk property, not a per-block query. Check the complete volume
		// before mutating anything; the epoch guard below detects a concurrent map reset.
		int regionSize = HostCollision.REGION_SIZE;
		for (int rx = Math.floorDiv(x0, regionSize); rx <= Math.floorDiv(x1, regionSize); rx++) {
			for (int ry = Math.floorDiv(y0, regionSize); ry <= Math.floorDiv(y1, regionSize); ry++) {
				for (int rz = Math.floorDiv(z0, regionSize); rz <= Math.floorDiv(z1, regionSize); rz++) {
					if (!HostCollision.isKnown(rx * regionSize, ry * regionSize, rz * regionSize)) {
						player.sendSystemMessage(Component.literal("Voxel sandbox: wait for collision to stream, then retry."));
						return true;
					}
				}
			}
		}
		for (int cx = x0 >> 4; cx <= (x1 >> 4); cx++) {
			for (int cz = z0 >> 4; cz <= (z1 >> 4); cz++) {
				if (!player.level().getChunkSource().hasChunk(cx, cz)) {
					player.sendSystemMessage(Component.literal("Voxel sandbox: wait for chunks to load, then retry."));
					return true;
				}
			}
		}
		AABB playerBox = player.getBoundingBox().inflate(0.05);
		List<Edit> edits = new ArrayList<>();
		for (int x = x0; x <= x1; x++) {
			for (int y = y0; y <= y1; y++) {
				for (int z = z0; z <= z1; z++) {

					BlockPos pos = new BlockPos(x, y, z);
					BlockState before = player.level().getBlockState(pos);
					// Never overwrite containers, player builds or fluids. All conversion edits are transient.
					if (!before.isAir()) {
						continue;
					}
					boolean shell = x == x0 || x == x1 || y == y0 || y == y1 || z == z0 || z == z1;
					boolean solid = HostCollision.solidFraction(pos) >= 0.05F && !HostCollision.isSkyAt(x, y, z);
					if (!shell && !solid) {
						continue;
					}
					if (x + 1 > playerBox.minX && x < playerBox.maxX && y + 1 > playerBox.minY && y < playerBox.maxY
						&& z + 1 > playerBox.minZ && z < playerBox.maxZ) {
						continue;
					}
					BlockState after = (shell ? Blocks.BEDROCK : Blocks.STONE).defaultBlockState();
					edits.add(new Edit(pos, before, after));
				}
			}
		}
		if (HostCollision.epoch() != epoch) {
			player.sendSystemMessage(Component.literal("Voxel sandbox: map collision changed; wait and retry."));
			return true;
		}
		cancel = false;
		job = new Job(player, new AABB(x0, y0, z0, x1 + 1, y1 + 1, z1 + 1), edits, epoch);
		player.sendSystemMessage(Component.literal("Voxel sandbox: converting a 25 x 17 x 25 area. Use a pickaxe when conversion completes."));
		return true;
	}

	private static void tick(MinecraftServer server) {
		if (cancel) {
			restore();
			cancel = false;
			return;
		}
		Job current = job;
		if (current != null && current.restoring) {
			restoreBatch(current, false);
			return;
		}
		if (current != null && (current.epoch != HostCollision.epoch() || current.player.isRemoved() || current.player.level() != current.level)) {
			restore();
			return;
		}
		if (current == null || current.ready) {
			return;
		}
		if (current.player.position().distanceToSqr(current.start) > 0.25) {
			restore();
			current.player.sendSystemMessage(Component.literal("Voxel sandbox cancelled: stand still while converting."));
			return;
		}
		int end = Math.min(current.edits.size(), current.cursor + BUDGET);
		long began = System.nanoTime();
		while (current.cursor < end && System.nanoTime() - began < TICK_BUDGET_NS) {
			Edit edit = current.edits.get(current.cursor++);
			if (current.level.getBlockState(edit.pos()) != edit.before()) {
				current.cursor--;
				restore();
				current.player.sendSystemMessage(Component.literal("Voxel sandbox cancelled: terrain changed during conversion."));
				return;
			}
			if (!current.level.setBlock(edit.pos(), edit.after(), Block.UPDATE_ALL)) {
				current.cursor--;
				restore();
				current.player.sendSystemMessage(Component.literal("Voxel sandbox cancelled: a block update was rejected."));
				return;
			}
		}
		if (current.cursor == current.edits.size()) {
			if (cancel || current.epoch != HostCollision.epoch()) {
				restore();
				return;
			}
			current.ready = true;
			active = current.bounds;
			current.player.sendSystemMessage(Component.literal("Voxel sandbox ready: stone is mineable, drops cobblestone and uses vanilla cracks/particles. Stop: hc_mc halfcraft_voxelize stop"));
		}
	}

	private static void restore() {
		active = null;
		if (job != null) job.restoring = true;
	}

	private static void restoreBatch(Job current, boolean shutdown) {
		long began = System.nanoTime();
		int count = 0;
		// Shutdown must finish before rollback/save teardown. Ordinary stops/cancellations have
		// the same bounded per-tick work as conversion, not thousands of neighbour updates at once.
		while (current.cursor > 0 && (shutdown || (count < BUDGET && System.nanoTime() - began < TICK_BUDGET_NS))) {
			Edit edit = current.edits.get(--current.cursor);
			current.level.setBlock(edit.pos(), edit.before(), Block.UPDATE_ALL);
			count++;
		}
		if (current.cursor == 0) job = null;
	}
}
