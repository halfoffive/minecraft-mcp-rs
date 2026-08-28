//! Periodic snapshot updates driven by bot Tick events.
//!
//! [`SnapshotUpdater`] encapsulates the throttled world-state collection
//! logic: reading bot position/health/gamemode, scanning dirty blocks,
//! and atomically updating [`SharedState`] via [`WorldSnapshot`].
//!
//! # Snapshot invariants
//!
//! Production snapshots **never contain air entries** — a broken block is
//! simply absent from `blocks`/`block_index`. Both refresh paths (dirty-block
//! single reads and dirty-chunk full scans) apply the same `is_air()` filter,
//! so the same world state always produces the same snapshot shape.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use azalea::Client;
use tracing::{debug, warn};

use crate::bot::commands::canonical_player_inventory;
use crate::snapshot::{DirtyTracker, SnapshotBuilder};
use crate::state::SharedState;
use crate::types::{
    BlockEntry, BlockPos, EntityEntry, GameMode, InventorySlot, SelfPlayer, UNKNOWN_ENTITY_ID,
    WorldSnapshot,
};
use crate::utils::to_snake_case;

// ═══════════════════════════════════════════════════════════════
// SnapshotUpdater
// ═══════════════════════════════════════════════════════════════

/// Manages throttled world-snapshot updates driven by bot Tick events.
///
/// The updater is designed to be created once and reused across ticks.
/// It holds shared references to the application state and dirty tracker
/// so that multiple event handlers can coordinate (e.g. chunk-receive
/// events mark chunks dirty, Tick events consume them).
pub struct SnapshotUpdater {
    shared_state: Arc<SharedState>,
    dirty_tracker: Arc<Mutex<DirtyTracker>>,
    last_update: Arc<Mutex<Instant>>,
    interval_ms: u64,
}

impl SnapshotUpdater {
    /// Create a new updater.
    ///
    /// All four parameters are typically extracted from
    /// [`BotState`](crate::bot::events::BotState) so that the updater shares
    /// the same state, tracker, and timer as the event loop.
    pub fn new(
        shared_state: Arc<SharedState>,
        dirty_tracker: Arc<Mutex<DirtyTracker>>,
        last_update: Arc<Mutex<Instant>>,
        interval_ms: u64,
    ) -> Self {
        Self {
            shared_state,
            dirty_tracker,
            last_update,
            interval_ms,
        }
    }

    /// Mark a single block position as dirty so that the next snapshot
    /// re-reads it from the world.
    pub fn mark_block_dirty(&self, pos: BlockPos) {
        self.dirty_tracker
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .mark_block_dirty(pos);
    }

    /// Mark an entire chunk as dirty so that the next snapshot re-reads
    /// all blocks in that chunk.
    pub fn mark_chunk_dirty(&self, chunk: (i32, i32)) {
        self.dirty_tracker
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .mark_chunk_dirty(chunk);
    }

    // ── Throttling ──────────────────────────────────────────

    /// Returns `true` if enough time has passed since the last update.
    /// Resets the timer on success so the caller does not need to.
    ///
    /// `pub(crate)`: `handle_tick` calls this **before** spawning the
    /// build task so a throttled tick never spawns a wasted task (azalea
    /// fires ~20 ticks/sec against a 500 ms snapshot interval).
    pub(crate) fn check_and_update_timer(&self) -> bool {
        let mut last = self.last_update.lock().unwrap_or_else(|e| e.into_inner());
        if last.elapsed() >= Duration::from_millis(self.interval_ms) {
            *last = Instant::now();
            true
        } else {
            false
        }
    }

    /// Rewind the throttle timer after a failed build (F-31).
    ///
    /// `check_and_update_timer` must arm the gate before the build task is
    /// spawned, otherwise every throttled tick (~18 of 20 per second) would
    /// spawn a wasted task. But a failed build used to leave that full
    /// interval in place, so the next attempt had to wait an entire snapshot
    /// period. This schedules the retry after only
    /// [`SNAPSHOT_BUILD_RETRY_DELAY`].
    pub(crate) fn schedule_retry_after_failure(&self) {
        let rewind =
            Duration::from_millis(self.interval_ms).saturating_sub(SNAPSHOT_BUILD_RETRY_DELAY);
        *self.last_update.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now() - rewind;
    }

    // ── Main tick handler ───────────────────────────────────

    /// Build a fresh snapshot and store it in [`SharedState`], moving the
    /// snapshot in (no clone — the previous `update_from_tick` cloned the
    /// whole snapshot just to return it; the caller only needs a bool).
    ///
    /// Returns `true` if a new snapshot was built and stored (callers use
    /// this to trigger a UI repaint).
    ///
    /// The caller is expected to have passed the
    /// `check_and_update_timer` gate first
    /// — this method does not throttle.
    pub async fn build_and_store(&self, bot: &Client) -> bool {
        match build_snapshot_inner(bot, &self.shared_state, &self.dirty_tracker).await {
            Ok(snapshot) => {
                self.shared_state.update_snapshot(snapshot);
                debug!("snapshot updated via SnapshotUpdater");
                true
            }
            Err(e) => {
                warn!("snapshot build failed: {e}");
                // F-31: don't make the next attempt wait a full snapshot
                // interval — retry after the short failure delay.
                self.schedule_retry_after_failure();
                false
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// Inner snapshot builder (free function — testable in isolation)
// ═══════════════════════════════════════════════════════════════

/// Per-build cap on fully-scanned dirty chunks (report M-25).
///
/// A full chunk scan costs up to 98,304 get_block_state calls and the
/// entire scan loop runs inside a single future with no await — an
/// unbounded first-login burst (~289 chunks at the default radius) stalled
/// the LocalSet for hundreds of milliseconds, freezing the command
/// executor and azalea event handlers. Chunks beyond the cap stay dirty in
/// the tracker and are scanned (nearest-first) on the next build.
const MAX_DIRTY_CHUNKS_PER_BUILD: usize = 32;

/// Retry delay after a failed snapshot build (F-31).
const SNAPSHOT_BUILD_RETRY_DELAY: Duration = Duration::from_millis(250);

/// A dirty-chunk coordinate pair (report M-25 scan planning).
type ChunkPair = (i32, i32);

/// Plan one build's dirty-chunk scan (report M-25): keep only chunks within
/// `chunk_scan_radius` of the player chunk, order them nearest-first (ties
/// broken deterministically by coordinates), and cap at
/// [`MAX_DIRTY_CHUNKS_PER_BUILD`]. Returns `(to_scan, deferred)` — the
/// deferred chunk set stays dirty on the tracker and is scanned (again
/// nearest-first) on the next build. Pure so the scheduling is
/// unit-testable without a bot.
fn plan_dirty_chunk_scan(
    dirty_chunks: &std::collections::HashSet<ChunkPair>,
    player_chunk: ChunkPair,
    chunk_scan_radius: i32,
) -> (Vec<ChunkPair>, Vec<ChunkPair>) {
    let mut in_radius: Vec<ChunkPair> = dirty_chunks
        .iter()
        .copied()
        .filter(|&(cx, cz)| {
            ((cx - player_chunk.0).abs()).max((cz - player_chunk.1).abs()) <= chunk_scan_radius
        })
        .collect();
    in_radius.sort_by_key(|&(cx, cz)| {
        (
            (cx - player_chunk.0).abs().max((cz - player_chunk.1).abs()),
            cx,
            cz,
        )
    });
    let deferred = in_radius.split_off(MAX_DIRTY_CHUNKS_PER_BUILD.min(in_radius.len()));
    (in_radius, deferred)
}

/// Absolute Y of a chunk section's bottom edge, derived from the
/// dimension's actual `min_y` instead of the overworld default.
fn section_base_y(world_min_y: i32, section_idx: usize) -> i32 {
    world_min_y + (section_idx as i32) * 16
}

fn block_within_chunk_radius(pos: BlockPos, player_chunk: (i32, i32), radius: i32) -> bool {
    let chunk = (pos.x >> 4, pos.z >> 4);
    ((chunk.0 - player_chunk.0).abs()).max((chunk.1 - player_chunk.1).abs()) <= radius
}

async fn build_snapshot_inner(
    bot: &Client,
    shared_state: &SharedState,
    dirty_tracker: &Arc<Mutex<DirtyTracker>>,
) -> eyre::Result<WorldSnapshot> {
    // ── Read bot components ──────────────────────────────────
    let position = bot.component::<azalea::entity::Position>();
    let health = bot.component::<azalea::entity::metadata::Health>();
    let hunger = bot.hunger();
    let local_gamemode = bot.component::<azalea::local_player::LocalGameMode>();
    let profile = bot.profile();

    // Sub-block-precision position. The integer `BlockPos` is used for
    // pathfinding and block lookups, but the top-down renderer needs the
    // exact floating-point coordinates to place the player marker at the
    // correct sub-pixel offset (otherwise the centre can be off by up to
    // 1 block — see the `position_precise` regression test).
    let position_precise: [f64; 3] = [position.x, position.y, position.z];

    // Player heading (yaw). Minecraft's yaw convention is:
    //   0      = facing +Z (south)
    //   +π/2   = facing -X (west)
    //   ±π     = facing -Z (north)
    //   -π/2   = facing +X (east)
    // The renderer converts yaw to a (dx, dz) screen offset for the
    // heading arrow.
    //
    // `azalea::entity::LookDirection` exposes `y_rot()` which returns the
    // player's horizontal look angle in radians. It is `None` only briefly
    // before the first `ClientboundPlayerLookAtPacket` is processed (e.g.
    // very first tick after spawn). The raw angle is folded into
    // Minecraft's `[-180, 180)` degree range here — the single write point
    // for `SelfPlayer::yaw` — so `get_bot_status` and the `get_world_view`
    // annotation never expose unbounded accumulated turns (e.g. -767.1°).
    // The checked wrapper also folds non-finite angles (NaN/±∞ have no
    // direction) to `None` — "yaw unknown" beats a poisoned annotation
    // (2026-08-26 review).
    let yaw: Option<f32> = bot
        .get_component::<azalea::entity::LookDirection>()
        .and_then(|look| crate::utils::normalize_yaw_checked(look.y_rot()));

    let inventory = read_inventory(bot);

    let self_player = SelfPlayer {
        uuid: profile.uuid.to_string(),
        username: profile.name,
        position: BlockPos::new(
            position.x.floor() as i32,
            position.y.floor() as i32,
            position.z.floor() as i32,
        ),
        health: health.0,
        hunger: hunger.food as i32,
        gamemode: azalea_gamemode_to_ours(local_gamemode.current),
        held_item_slot: bot.selected_hotbar_slot(),
        inventory,
        position_precise: Some(position_precise),
        yaw,
    };

    // ── Read old snapshot ────────────────────────────────────
    let old_snapshot = shared_state.read_snapshot();

    // ── Drain dirty sets ─────────────────────────────────────
    let (dirty_blocks, dirty_chunks) = {
        let mut tracker = dirty_tracker.lock().unwrap_or_else(|e| e.into_inner());
        tracker.take_dirty_sets()
    };

    // M-7: Player chunk and scan radius, used to skip far-away dirty
    // chunks. `chunk_scan_radius` (default 8, range 1–16) limits how many
    // chunks around the player are fully scanned on each snapshot tick.
    let player_chunk = (self_player.position.x >> 4, self_player.position.z >> 4);
    let chunk_scan_radius = shared_state.read_config().chunk_scan_radius as i32;
    // Snapshot retention bound. `chunk_scan_radius` controls how far freshly
    // loaded chunks are scanned, but the snapshot also serves `get_nearby_blocks`
    // (max radius 100 blocks ≈ 7 chunks), so at least 8 chunks around the
    // player are always retained. Old blocks outside this radius are pruned
    // on every build — without this, every chunk the bot ever walked through
    // stayed in `blocks` forever and the per-tick clone + block_index rebuild
    // grew without bound.
    let retention_chunks = chunk_scan_radius.max(8);

    // ── Read world for changed blocks ────────────────────────
    let mut new_blocks = Vec::new();

    // M-25: filter the dirty chunks to the scan radius, order them
    // deterministically nearest-first, and cap the per-build count. The
    // whole chunk-scan loop below runs inside one future with NO await, so
    // an unbounded first-login burst (~289 chunks) stalled the LocalSet
    // for hundreds of milliseconds — the serial executor, azalea events,
    // everything behind it. Chunks beyond the cap stay dirty and are
    // scanned (nearest-first) on the next build. Declared OUTSIDE the
    // "if" block so the builder tracker (after the scan) can mark exactly
    // the chunks processed this build.
    let (in_radius_chunks, overflow_chunks) =
        plan_dirty_chunk_scan(&dirty_chunks, player_chunk, chunk_scan_radius);

    if !dirty_blocks.is_empty() || !dirty_chunks.is_empty() {
        let world = bot.world();
        let world_guard = world.read();
        // F-11: the section base must come from the dimension's actual
        // ChunkStorage, not the overworld default -64. Custom dimensions can
        // shift the whole vertical range and the previous hardcoded value made
        // every scanned BlockPos (and therefore the snapshot) vertically
        // offset for those dimensions.
        let world_min_y = world_guard.chunks.min_y;

        // M-10: Read individual dirty blocks (from block-update events)
        // rather than scanning their entire chunk. Skip blocks whose chunk
        // is in `dirty_chunks` AND within scan radius — those chunks will be
        // fully scanned below, so reading individual blocks here would be
        // redundant (each full scan is 98,304 `get_block_state` calls).
        // Blocks in far-away dirty chunks (outside radius) are still read
        // here because the full scan is skipped for them, so this is the
        // only chance to refresh their state in the snapshot.
        for pos in &dirty_blocks {
            let chunk = (pos.x >> 4, pos.z >> 4);
            // Prune dirty block updates that fall outside the retention
            // radius too — they would be dropped by the old-block filter
            // below, so reading them here would be wasted work.
            if !block_within_chunk_radius(*pos, player_chunk, retention_chunks) {
                continue;
            }
            if dirty_chunks.contains(&chunk) {
                let dist = ((chunk.0 - player_chunk.0).abs()).max((chunk.1 - player_chunk.1).abs());
                if dist <= chunk_scan_radius {
                    continue;
                }
            }
            let az_pos = azalea::core::position::BlockPos::new(pos.x, pos.y, pos.z);
            if let Some(block_state) = world_guard.get_block_state(az_pos) {
                // Invariant: production snapshots never contain air entries —
                // a broken block is simply absent (same filter as the
                // dirty-chunk scan below, so both paths produce identical
                // snapshot shapes for the same world state). No consumer
                // depends on air entries: `wait_for_block_gone`/`wait_for_block_present`
                // treat absent == air, `find_standable_neighbor` treats
                // absent as free, and `get_nearby_blocks` had to filter them.
                if block_state.is_air() {
                    continue;
                }
                let block_name = block_state_to_name(block_state);
                new_blocks.push(BlockEntry {
                    position: *pos,
                    block_type: block_name,
                    block_state: None,
                });
            }
        }

        // M-7: Only scan dirty chunks within chunk_scan_radius of the
        // player's current chunk. Chunks outside this radius are skipped to
        // avoid expensive 98,304-position scans for far-away chunks.
        //
        // M-25: the scan additionally bounds the number of chunks per build
        // (MAX_DIRTY_CHUNKS_PER_BUILD, nearest first). The whole scan runs
        // inside one future with NO await, so an unbounded first-login burst
        // (~289 chunks) stalled the LocalSet for hundreds of milliseconds —
        // the serial executor, azalea events, everything behind it. Chunks
        // beyond the cap stay dirty and are scanned on the next build.
        //
        // Scan each dirty chunk in full so the blocks inside it are re-read
        // from the world. SnapshotBuilder::build() removes every block whose
        // chunk is in the builder tracker below, so without re-adding the
        // surviving blocks here they would be permanently lost.
        //
        // Chunk dimensions for Minecraft 1.21 are 16×384×16: x and z span
        // 16 blocks within the chunk, y spans the full build height
        // (-64..320). Only non-air blocks are recorded to avoid bloating
        // the snapshot with empty positions.
        //
        // Section-level scan optimisation: a chunk has 24 vertical sections
        // (16×16×16 = 4096 blocks each). Before scanning a section, we read
        // its block_count (number of non-air blocks) from the chunk's
        // sections array. If block_count == 0, the entire section is air
        // and we skip its 4096 get_block_state calls — for typical surface
        // chunks this skips ~23/24 sections, reducing the per-chunk scan
        // (in_radius_chunks / overflow_chunks are declared before the
        // "if" block so the builder tracker below still sees them.)

        for &(chunk_x, chunk_z) in &in_radius_chunks {
            let base_x = chunk_x * 16;
            let base_z = chunk_z * 16;

            // Build the list of section indices to scan. We hold the
            // chunk's read lock just long enough to read each section's
            // block_count, then release before the per-block scan
            // (which acquires its own lock through world.get_block_state).
            let chunk_pos = azalea::core::position::ChunkPos::new(chunk_x, chunk_z);
            let sections_to_scan: Vec<usize> =
                if let Some(chunk_arc) = world_guard.chunks.get(&chunk_pos) {
                    let chunk_guard = chunk_arc.read();
                    chunk_guard
                        .sections
                        .iter()
                        .enumerate()
                        .filter(|(_, s)| s.block_count > 0)
                        .map(|(i, _)| i)
                        .collect()
                } else {
                    // Chunk not loaded (shouldn't happen for a dirty chunk, but
                    // be defensive). M-25: do NOT fall back to scanning all 24
                    // sections — an unloaded chunk has no blocks to read, so
                    // the fallback only wasted 98,304 get_block_state calls.
                    // The chunk's old blocks are removed from the snapshot by
                    // the builder tracker below, and a later chunk-load event
                    // re-marks the chunk dirty so the reloaded blocks come back.
                    debug!(chunk_x, chunk_z, "dirty chunk not loaded — skipping scan");
                    Vec::new()
                };

            for section_idx in sections_to_scan {
                // With the overworld default (min_y=-64, 24 sections) this
                // covers y=-64..-48, ..., y=304..320.
                let section_min_y = section_base_y(world_min_y, section_idx);
                for dx in 0..16i32 {
                    for dy in 0..16i32 {
                        for dz in 0..16i32 {
                            let pos = BlockPos::new(base_x + dx, section_min_y + dy, base_z + dz);
                            let az_pos = azalea::core::position::BlockPos::new(pos.x, pos.y, pos.z);
                            if let Some(block_state) = world_guard.get_block_state(az_pos) {
                                if block_state.is_air() {
                                    continue;
                                }
                                let block_name = block_state_to_name(block_state);
                                new_blocks.push(BlockEntry {
                                    position: pos,
                                    block_type: block_name,
                                    block_state: None,
                                });
                            }
                        }
                    }
                }
            }
        }

        // M-25: chunks deferred past the per-build cap stay dirty so the
        // next build scans them (nearest-first). They are NOT marked in
        // the builder tracker — their old blocks stay in the snapshot
        // (stale but consistent) until the deferred scan runs.
        if !overflow_chunks.is_empty() {
            debug!(
                count = overflow_chunks.len(),
                "dirty-chunk scan capped; deferring to the next snapshot build"
            );
            let mut tracker = dirty_tracker.lock().unwrap_or_else(|e| e.into_inner());
            for &(chunk_x, chunk_z) in &overflow_chunks {
                tracker.mark_chunk_dirty((chunk_x, chunk_z));
            }
        }
        drop(world_guard);
    }

    // ── Repopulate tracker for SnapshotBuilder ───────────────
    // M-7: Only mark chunks within scan radius as dirty in the builder
    // tracker. Far-away dirty chunks are not scanned, and their old blocks
    // are preserved (stale but acceptable for far-away regions). Individual
    // dirty blocks are always marked so their old versions are removed and
    // the freshly-read versions are added.
    //
    // M-25: only the chunks ACTUALLY processed this build are marked — a
    // chunk deferred past the per-build cap keeps its old blocks (stale but
    // consistent) and stays dirty on the main tracker (re-marked inside the
    // scan block above) so the next build scans it.
    //
    // F-26: individual dirty blocks inside a deferred (overflow) chunk were
    // neither read above (the full scan was deferred) nor covered by the
    // chunk-level mark. Marking them here used to delete their old snapshot
    // entries with no replacement, making changed blocks "disappear" for one
    // build. Keep those old entries until the deferred scan actually runs.
    let overflow_set: std::collections::HashSet<ChunkPair> =
        overflow_chunks.iter().copied().collect();
    let mut builder_tracker = DirtyTracker::new();
    for pos in &dirty_blocks {
        let chunk = (pos.x >> 4, pos.z >> 4);
        if overflow_set.contains(&chunk) {
            continue;
        }
        builder_tracker.mark_block_dirty(*pos);
    }
    for &(chunk_x, chunk_z) in &in_radius_chunks {
        builder_tracker.mark_chunk_dirty((chunk_x, chunk_z));
    }
    // Only the old snapshot's blocks are carried into the builder — the
    // old snapshot (incl. its block_index HashMap) is not deep-cloned.
    // Before carrying them over, prune blocks outside retention_chunks so
    // the snapshot (and its O(1) block_index) stays bounded no matter how
    // far the bot travels.
    let retained_old_blocks: Vec<BlockEntry> = old_snapshot
        .blocks
        .iter()
        .filter(|b| block_within_chunk_radius(b.position, player_chunk, retention_chunks))
        .cloned()
        .collect();
    new_blocks.retain(|b| block_within_chunk_radius(b.position, player_chunk, retention_chunks));

    let mut builder = SnapshotBuilder::new(retained_old_blocks)
        .with_dirty_tracker(&mut builder_tracker)
        .with_self_player(self_player);

    if !new_blocks.is_empty() {
        builder = builder.with_blocks(new_blocks);
    }

    // ── Chunk summary from partial world ─────────────────────
    // azalea 0.15.1: WorldHolder was renamed to InstanceHolder, and its
    // field `partial` was renamed to `partial_instance`.
    let chunk_summary =
        if let Some(world_holder) = bot.get_component::<azalea::local_player::InstanceHolder>() {
            let partial_world = world_holder.partial_instance.read();
            let storage = &partial_world.chunks;
            storage
                .chunks()
                .enumerate()
                .filter_map(|(i, chunk)| {
                    chunk.as_ref().map(|_| {
                        let pos = storage.chunk_pos_from_index(i);
                        (pos.x, pos.z)
                    })
                })
                .collect()
        } else {
            old_snapshot.chunk_summary.clone()
        };

    builder = builder.with_chunk_summary(chunk_summary);

    // ── Entities from the live ECS ──────────────────────────
    // F6-2: entities are rebuilt from the live ECS on every snapshot so
    // positions, types and health stay current. Previously the list was
    // only seeded from AddPlayer events (players only, frozen at their
    // join position), so `collect_items` never saw dropped items and
    // `get_nearby_entities` never saw mobs.
    let entities = collect_entities(bot);
    builder = builder.with_entities(Some(entities));

    let mut snapshot = builder.build();
    // Populate `commands_enabled` — a live `/seed` probe result wins over the
    // permission-level heuristic when one exists (see
    // `resolve_commands_enabled`). OP level > 0 means commands are enabled;
    // == 0 means disabled; unavailable (component not yet present) means
    // unknown.
    snapshot.commands_enabled = resolve_commands_enabled(bot, shared_state);
    Ok(snapshot)
}

// ═══════════════════════════════════════════════════════════════
// Utility helpers
// ═══════════════════════════════════════════════════════════════

/// Read the player's 36-slot inventory from the azalea client in canonical
/// order (hotbar 0-8 first, main inventory 9-35).
///
/// Delegates to [`crate::bot::commands::canonical_player_inventory`] so the
/// slot indices match every other inventory consumer; the previous inline
/// implementation read `Menu::Player.inventory` verbatim (main-inventory-first
/// protocol order) and returned an empty list while a container was open.
/// Only non-empty slots are returned.
fn read_inventory(bot: &Client) -> Vec<InventorySlot> {
    canonical_player_inventory(&bot.menu())
        .into_iter()
        .enumerate()
        .filter_map(|(slot, stack)| {
            stack.map(|s| InventorySlot {
                slot_index: slot as u8,
                item_id: s.item_id,
                count: s.count,
            })
        })
        .collect()
}

/// Convert an azalea [`EntityKind`] into the snake_case entity type string
/// exposed in snapshots and MCP tools (`"player"`, `"item"`, `"zombie"`,
/// ...).
///
/// The `EntityKind` enum variant names are PascalCase (matching the Java
/// registry names), so the shared [`to_snake_case`] helper produces the
/// same naming convention used for block and item ids. `None` (the entity
/// kind component is not present yet) maps to `"unknown"` so half-spawned
/// entities are still listed instead of being silently dropped.
///
/// [`EntityKind`]: azalea::registry::builtin::EntityKind
pub(crate) fn entity_type_string(kind: Option<azalea::registry::builtin::EntityKind>) -> String {
    match kind {
        Some(kind) => to_snake_case(&format!("{kind:?}")),
        None => "unknown".to_string(),
    }
}

/// Rebuild the entity list from the live ECS world.
///
/// azalea keeps two complementary indexes for the entities in the bot's
/// dimension:
///
/// - [`Instance::entity_by_id`] maps Minecraft entity ids to ECS entities
///   (maintained by azalea's indexing systems on spawn/despawn);
/// - the ECS world itself holds the actual components
///   ([`Position`], [`EntityUuid`], [`EntityKindComponent`],
///   [`Health`], [`GameProfileComponent`] for players).
///
/// The id index is copied first (holding only the instance read lock),
/// that lock is dropped, and only then is the ECS walked once. Holding
/// both locks at the same time could deadlock with azalea's schedule
/// loop, which holds the ECS lock while its systems acquire instance
/// write locks.
///
/// The bot's own entity is skipped — it is fully represented by
/// [`SelfPlayer`], and including it would double-draw the player marker
/// in the top-down world view. Entities are limited to the bot's current
/// dimension via [`InstanceName`].
///
/// [`Instance::entity_by_id`]: azalea::world::Instance
/// [`Position`]: azalea::entity::Position
/// [`EntityUuid`]: azalea::entity::EntityUuid
/// [`EntityKindComponent`]: azalea::entity::EntityKindComponent
/// [`Health`]: azalea::entity::metadata::Health
/// [`GameProfileComponent`]: azalea::player::GameProfileComponent
/// [`InstanceName`]: azalea::world::InstanceName
fn collect_entities(bot: &Client) -> Vec<EntityEntry> {
    let Some(holder) = bot.get_component::<azalea::local_player::InstanceHolder>() else {
        return Vec::new();
    };
    let Some(instance_name) = bot.get_component::<azalea::world::InstanceName>() else {
        return Vec::new();
    };

    // Minecraft entity id by ECS entity, copied under the instance read
    // lock only (the lock is dropped before the ECS is touched — see the
    // deadlock note above).
    let id_by_entity: HashMap<azalea::ecs::entity::Entity, i32> = {
        let instance = holder.instance.read();
        instance
            .entity_by_id
            .iter()
            .map(|(mc_id, entity)| (*entity, mc_id.0))
            .collect()
    };

    let local_entity = bot.entity;
    let mut entries = Vec::new();

    let mut ecs = bot.ecs.lock();
    let mut query = ecs.query::<(
        azalea::ecs::entity::Entity,
        &azalea::world::InstanceName,
        &azalea::entity::Position,
        &azalea::entity::EntityUuid,
        Option<&azalea::entity::EntityKindComponent>,
        Option<&azalea::entity::metadata::Health>,
        Option<&azalea::player::GameProfileComponent>,
    )>();
    for (entity, entity_instance, position, uuid, kind, health, profile) in query.iter(&ecs) {
        if *entity_instance != instance_name || entity == local_entity {
            continue;
        }
        // Skip corpses: a dead mob lingers in the ECS (health metadata 0.0)
        // until the server sends RemoveEntities / azalea's despawn cleanup
        // runs, which can take a second or more. Listing it as a live entity
        // made `get_nearby_entities` report dead mobs with `health: 0.0`.
        // Item entities carry no Health component, so drops are unaffected.
        if let Some(h) = health
            && h.0 <= 0.0
        {
            continue;
        }
        // F-27: a missing entity ID must not collapse to 0 — id 0 may be a
        // real entity in the index, and `attack_entity(0)` would be ambiguous.
        let minecraft_id = id_by_entity.get(&entity).copied();
        entries.push(EntityEntry {
            id: minecraft_id
                .and_then(|mc_id| u32::try_from(mc_id).ok())
                .unwrap_or(UNKNOWN_ENTITY_ID),
            uuid: (**uuid).to_string(),
            entity_type: entity_type_string(kind.map(|k| k.0)),
            position: BlockPos::new(
                position.x.floor() as i32,
                position.y.floor() as i32,
                position.z.floor() as i32,
            ),
            display_name: profile.map(|p| p.0.name.clone()),
            health: health.map(|h| h.0),
        });
    }
    drop(ecs);

    entries
}

/// Read whether server commands are enabled for the bot.
///
/// Returns `Some(true)` if the player's permission level is > 0 (OP),
/// `Some(false)` if it's 0 (no OP), or `None` if the `PermissionLevel`
/// component isn't available yet (e.g. before joining the world).
fn read_commands_enabled(bot: &Client) -> Option<bool> {
    bot.get_component::<azalea::local_player::PermissionLevel>()
        .map(|level| level.0 > 0)
}

/// Resolve `commands_enabled` for the snapshot.
///
/// A live `/seed` probe result (set by `get_server_info` after a round-trip)
/// takes precedence — on vanilla servers `PermissionLevel` correlates with
/// command ability, but cheat/plugin servers often let non-OP players run
/// commands, so the probe (which observes the server's actual reply) is the
/// truthful source. Falls back to the permission-level heuristic while
/// unprobed.
fn resolve_commands_enabled(bot: &Client, state: &SharedState) -> Option<bool> {
    state
        .get_commands_probe()
        .or_else(|| read_commands_enabled(bot))
}

/// Cache of resolved block names, keyed by the raw [`BlockState`] id.
///
/// A dirty-chunk scan re-reads thousands of blocks per snapshot tick, and
/// each block name used to pay `format!("{kind:?}")` + `to_snake_case()` —
/// two heap allocations per block, every tick. `BlockState` is a `Copy`
/// `Hash` id, so the parse cost is now paid once per distinct block state
/// instead of once per block.
static BLOCK_NAME_CACHE: LazyLock<Mutex<HashMap<azalea::block::BlockState, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn block_state_to_name(block_state: azalea::block::BlockState) -> String {
    {
        let cache = BLOCK_NAME_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(name) = cache.get(&block_state) {
            return name.clone();
        }
    }
    // Miss: compute outside the lock (the Debug formatting below allocates).
    #[allow(deprecated)]
    let block_kind = azalea::registry::Block::from(block_state);
    let debug_name = format!("{block_kind:?}");
    let name = to_snake_case(&debug_name);
    BLOCK_NAME_CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(block_state, name.clone());
    name
}

fn azalea_gamemode_to_ours(gm: azalea::core::game_type::GameMode) -> GameMode {
    match gm {
        azalea::core::game_type::GameMode::Survival => GameMode::Survival,
        azalea::core::game_type::GameMode::Creative => GameMode::Creative,
        azalea::core::game_type::GameMode::Adventure => GameMode::Adventure,
        azalea::core::game_type::GameMode::Spectator => GameMode::Spectator,
    }
}

// ═══════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use azalea::registry::builtin::EntityKind;

    // ── Helpers ─────────────────────────────────────────────

    fn make_updater() -> SnapshotUpdater {
        SnapshotUpdater::new(
            Arc::new(SharedState::new(AppConfig::default())),
            Arc::new(Mutex::new(DirtyTracker::new())),
            Arc::new(Mutex::new(Instant::now() - Duration::from_secs(3600))),
            500,
        )
    }

    fn make_updater_with_recent_timer() -> SnapshotUpdater {
        SnapshotUpdater::new(
            Arc::new(SharedState::new(AppConfig::default())),
            Arc::new(Mutex::new(DirtyTracker::new())),
            Arc::new(Mutex::new(Instant::now())),
            500,
        )
    }

    // ── Retention pruning ────────────────────────────────────

    /// A block that the server reports as air (e.g. after being broken) must
    /// disappear from the snapshot entirely: production snapshots never
    /// contain air entries — a broken block is simply absent. The dirty-block
    /// single-read path previously stored air entries while the dirty-chunk
    /// scan path skipped them, so the same world state produced different
    /// snapshot shapes depending on which path refreshed it.
    #[tokio::test]
    async fn test_dirty_block_update_to_air_removes_entry() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let tracker = Arc::new(Mutex::new(DirtyTracker::new()));

        // Old snapshot: a stone block at P.
        let p = BlockPos::new(5, 64, 5);
        state.update_snapshot(WorldSnapshot {
            blocks: vec![BlockEntry {
                position: p,
                block_type: "stone".into(),
                block_state: None,
            }],
            ..Default::default()
        });

        // Underlying world: chunk (0,0) loaded and entirely air.
        let mut chunk_storage = azalea::world::ChunkStorage::new(384, -64);
        let chunk = Arc::new(parking_lot::RwLock::new(azalea::world::Chunk::default()));
        chunk_storage.map.insert(
            azalea::core::position::ChunkPos::new(0, 0),
            Arc::downgrade(&chunk),
        );
        let instance = Arc::new(parking_lot::RwLock::new(azalea::world::Instance::from(
            chunk_storage,
        )));

        let mut world = bevy_ecs::world::World::new();
        let bot_entity = world.spawn(()).id();
        world.entity_mut(bot_entity).insert((
            azalea::entity::Position::new(azalea::core::position::Vec3::new(0.5, 64.0, 0.5)),
            azalea::entity::metadata::Health(20.0),
            azalea::local_player::Hunger::default(),
            azalea::local_player::LocalGameMode::from(azalea::core::game_type::GameMode::Survival),
            azalea::player::GameProfileComponent(azalea::auth::game_profile::GameProfile::new(
                uuid::Uuid::new_v4(),
                "AI_Bot".to_string(),
            )),
            azalea::local_player::InstanceHolder {
                instance: instance.clone(),
                partial_instance: Arc::new(parking_lot::RwLock::new(
                    azalea::world::PartialInstance::default(),
                )),
            },
            azalea::entity::inventory::Inventory::default(),
        ));
        let bot = azalea::Client::new(bot_entity, Arc::new(parking_lot::Mutex::new(world)));

        // Mark P dirty; the world now reports air there (default chunk).
        tracker
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .mark_block_dirty(p);

        let snapshot = build_snapshot_inner(&bot, &state, &tracker)
            .await
            .expect("build should succeed");

        // Invariant: a broken block is absent — no air entry, no index slot.
        assert!(
            !snapshot.blocks.iter().any(|b| b.position == p),
            "air entry for {p:?} must be filtered out, got: {:?}",
            snapshot
                .blocks
                .iter()
                .filter(|b| b.position == p)
                .collect::<Vec<_>>()
        );
        assert!(
            !snapshot.block_index.contains_key(&p),
            "block_index must not contain the removed block at {p:?}"
        );
    }

    /// F-11: section bases derive from the dimension's real `min_y`, so a
    /// custom dimension is not scanned with the overworld's -64 offset.
    #[test]
    fn test_section_base_y_uses_world_min_y() {
        assert_eq!(section_base_y(-64, 0), -64);
        assert_eq!(section_base_y(-64, 23), 304);
        assert_eq!(section_base_y(0, 0), 0);
        assert_eq!(section_base_y(0, 23), 368);
        assert_eq!(section_base_y(-16, 3), 32);
    }

    #[test]
    fn test_block_within_chunk_radius_prunes_far_chunks() {
        let player_chunk = (0, 0);
        // Inside the retention radius.
        assert!(block_within_chunk_radius(
            BlockPos::new(0, 64, 0),
            player_chunk,
            8
        ));
        assert!(block_within_chunk_radius(
            BlockPos::new(8 * 16, 64, 8 * 16),
            player_chunk,
            8
        ));
        // One chunk beyond the retention radius on either axis.
        assert!(!block_within_chunk_radius(
            BlockPos::new(9 * 16, 64, 0),
            player_chunk,
            8
        ));
        assert!(!block_within_chunk_radius(
            BlockPos::new(0, 64, 9 * 16),
            player_chunk,
            8
        ));
    }

    // ── Construction ────────────────────────────────────────

    #[test]
    fn test_new_creates_with_correct_interval() {
        let updater = SnapshotUpdater::new(
            Arc::new(SharedState::new(AppConfig::default())),
            Arc::new(Mutex::new(DirtyTracker::new())),
            Arc::new(Mutex::new(Instant::now())),
            250,
        );
        assert_eq!(updater.interval_ms, 250);
    }

    #[test]
    fn test_new_shares_state() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let updater = SnapshotUpdater::new(
            Arc::clone(&state),
            Arc::new(Mutex::new(DirtyTracker::new())),
            Arc::new(Mutex::new(Instant::now())),
            500,
        );
        assert!(Arc::ptr_eq(&updater.shared_state, &state));
    }

    #[test]
    fn test_new_shares_dirty_tracker() {
        let tracker = Arc::new(Mutex::new(DirtyTracker::new()));
        let updater = SnapshotUpdater::new(
            Arc::new(SharedState::new(AppConfig::default())),
            Arc::clone(&tracker),
            Arc::new(Mutex::new(Instant::now())),
            500,
        );
        assert!(Arc::ptr_eq(&updater.dirty_tracker, &tracker));
    }

    // ── Dirty marking ───────────────────────────────────────

    #[test]
    fn test_mark_block_dirty_adds_to_tracker() {
        let tracker = Arc::new(Mutex::new(DirtyTracker::new()));
        let updater = SnapshotUpdater::new(
            Arc::new(SharedState::new(AppConfig::default())),
            Arc::clone(&tracker),
            Arc::new(Mutex::new(Instant::now())),
            500,
        );
        updater.mark_block_dirty(BlockPos::new(10, 64, 20));
        let t = tracker.lock().unwrap();
        assert!(!t.is_empty());
    }

    #[test]
    fn test_mark_chunk_dirty_adds_to_tracker() {
        let tracker = Arc::new(Mutex::new(DirtyTracker::new()));
        let updater = SnapshotUpdater::new(
            Arc::new(SharedState::new(AppConfig::default())),
            Arc::clone(&tracker),
            Arc::new(Mutex::new(Instant::now())),
            500,
        );
        updater.mark_chunk_dirty((3, -7));
        let t = tracker.lock().unwrap();
        assert!(!t.is_empty());
    }

    #[test]
    fn test_multiple_dirty_marks_accumulate() {
        let tracker = Arc::new(Mutex::new(DirtyTracker::new()));

        let updater = SnapshotUpdater::new(
            Arc::new(SharedState::new(AppConfig::default())),
            Arc::clone(&tracker),
            Arc::new(Mutex::new(Instant::now())),
            500,
        );
        updater.mark_block_dirty(BlockPos::new(1, 0, 0));
        updater.mark_block_dirty(BlockPos::new(2, 0, 0));
        updater.mark_chunk_dirty((0, 0));
        let (blocks, chunks) = tracker.lock().unwrap().take_dirty_sets();
        assert_eq!(blocks.len(), 2);
        assert_eq!(chunks.len(), 1);
    }

    /// Report M-25: the dirty-chunk scan plan keeps only in-radius chunks,
    /// orders them nearest-first, and caps the per-build count — the
    /// deferred remainder must be re-scanned in a later build instead of
    /// stalling the LocalSet with hundreds of chunk scans in one future.
    #[test]
    fn test_plan_dirty_chunk_scan_nearest_first_and_capped() {
        use std::collections::HashSet;

        // Neighbourhood: in-radius chunks are sorted by Chebyshev distance
        // (ties by coordinates), far chunks are filtered out.
        let mut set = HashSet::new();
        set.insert((0, 0)); // distance 0
        set.insert((1, 0)); // distance 1
        set.insert((0, 2)); // distance 2
        set.insert((3, 0)); // distance 3
        set.insert((9, 0)); // outside radius 8 — filtered
        let (scan, deferred) = plan_dirty_chunk_scan(&set, (0, 0), 8);
        assert_eq!(scan, vec![(0, 0), (1, 0), (0, 2), (3, 0)]);
        assert!(deferred.is_empty());

        // Cap: 40 in-radius chunks -> 32 scanned (nearest first), 8 deferred
        // for the next build.
        let mut big = HashSet::new();
        for i in 0..40 {
            // 5 x positions (-2..=2) x 8 z positions (0..=7): 40 distinct,
            // all within Chebyshev distance 8 of (0,0).
            big.insert((i % 5 - 2, i / 5));
        }
        let (scan_big, deferred_big) = plan_dirty_chunk_scan(&big, (0, 0), 8);
        assert_eq!(scan_big.len(), MAX_DIRTY_CHUNKS_PER_BUILD);
        assert_eq!(deferred_big.len(), 40 - MAX_DIRTY_CHUNKS_PER_BUILD);
        // Nearest-first: the player's own chunk is the first entry.
        assert_eq!(scan_big[0], (0, 0));
        // Scan + deferred cover the whole in-radius set exactly once.
        let mut all = scan_big.clone();
        all.extend(deferred_big.iter().copied());
        all.sort();
        let mut expect = big.into_iter().collect::<Vec<_>>();
        expect.sort();
        assert_eq!(all, expect);
    }

    // ── Throttling ──────────────────────────────────────────

    // ── Throttling ──────────────────────────────────────────

    #[test]
    fn test_throttle_first_call_allows_update() {
        let updater = make_updater();
        // last_update is 3600s in the past, so first call should succeed
        assert!(updater.check_and_update_timer());
    }

    #[test]
    fn test_throttle_immediate_second_call_blocks() {
        let updater = make_updater();
        assert!(updater.check_and_update_timer()); // first: allowed
        assert!(!updater.check_and_update_timer()); // second: throttled
    }

    #[test]
    fn test_throttle_with_recent_timer_blocks() {
        let updater = make_updater_with_recent_timer();
        // last_update is now, so interval hasn't passed
        assert!(!updater.check_and_update_timer());
    }

    #[test]
    fn test_throttle_respects_custom_interval() {
        let updater = SnapshotUpdater::new(
            Arc::new(SharedState::new(AppConfig::default())),
            Arc::new(Mutex::new(DirtyTracker::new())),
            Arc::new(Mutex::new(Instant::now() - Duration::from_millis(100))),
            200, // interval: 200ms, elapsed: 100ms → throttled
        );
        assert!(!updater.check_and_update_timer());
    }

    #[test]
    fn test_throttle_allows_when_elapsed_exceeds_interval() {
        let updater = SnapshotUpdater::new(
            Arc::new(SharedState::new(AppConfig::default())),
            Arc::new(Mutex::new(DirtyTracker::new())),
            Arc::new(Mutex::new(Instant::now() - Duration::from_millis(600))),
            500, // interval: 500ms, elapsed: 600ms → allowed
        );
        assert!(updater.check_and_update_timer());
    }

    /// F-31: after a failed build the timer is rewound so the next attempt
    /// only waits the short retry delay — not a whole snapshot interval.
    #[test]
    fn test_failed_build_schedules_short_retry() {
        let updater = make_updater_with_recent_timer();
        updater.schedule_retry_after_failure();

        // The rewind is `interval - 250ms`, so the gate is still closed
        // immediately after the failure…
        assert!(!updater.check_and_update_timer());

        // …but the recorded last-update is strictly closer than a fresh
        // successful gate would have left it.
        let elapsed = {
            let last = updater
                .last_update
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            last.elapsed()
        };
        assert!(
            elapsed < Duration::from_millis(updater.interval_ms),
            "failed-build rewind must be shorter than a full interval, got {elapsed:?}"
        );
    }

    #[test]
    fn test_throttle_resets_timer_on_allow() {
        let updater = make_updater();
        assert!(updater.check_and_update_timer()); // allowed, timer reset
        // Now timer was just reset to now
        assert!(!updater.check_and_update_timer()); // throttled
    }

    // ── Utility functions ───────────────────────────────────

    #[test]
    fn test_azalea_gamemode_conversion() {
        assert_eq!(
            azalea_gamemode_to_ours(azalea::core::game_type::GameMode::Survival),
            GameMode::Survival
        );
        assert_eq!(
            azalea_gamemode_to_ours(azalea::core::game_type::GameMode::Creative),
            GameMode::Creative
        );
        assert_eq!(
            azalea_gamemode_to_ours(azalea::core::game_type::GameMode::Adventure),
            GameMode::Adventure
        );
        assert_eq!(
            azalea_gamemode_to_ours(azalea::core::game_type::GameMode::Spectator),
            GameMode::Spectator
        );
    }

    #[test]
    fn test_to_snake_case() {
        assert_eq!(to_snake_case("GrassBlock"), "grass_block");
        assert_eq!(to_snake_case("Stone"), "stone");
        assert_eq!(to_snake_case("OakPlanks"), "oak_planks");
        assert_eq!(to_snake_case("DiamondOre"), "diamond_ore");
    }

    // ── Entity type mapping (F6-2) ────────────────────────

    #[test]
    fn test_entity_type_string_player() {
        assert_eq!(entity_type_string(Some(EntityKind::Player)), "player");
    }

    #[test]
    fn test_entity_type_string_item() {
        assert_eq!(entity_type_string(Some(EntityKind::Item)), "item");
    }

    #[test]
    fn test_entity_type_string_zombie() {
        assert_eq!(entity_type_string(Some(EntityKind::Zombie)), "zombie");
    }

    #[test]
    fn test_entity_type_string_item_frame() {
        // Item frames are block-like entities and must NOT match the
        // collect_items filter (which excludes anything containing
        // "frame").
        assert_eq!(
            entity_type_string(Some(EntityKind::ItemFrame)),
            "item_frame"
        );
    }

    #[test]
    fn test_entity_type_string_unknown_when_kind_missing() {
        assert_eq!(entity_type_string(None), "unknown");
    }

    // ── collect_entities — live ECS rebuild (F6-2) ────────

    /// A client whose entity has no `InstanceHolder` (not fully joined
    /// yet) must yield an empty entity list without panicking. The bot
    /// entity itself must exist in the world — azalea's `query_self`
    /// panics for entities that are missing entirely.
    #[test]
    fn test_collect_entities_empty_world() {
        let mut world = bevy_ecs::world::World::new();
        let bot_entity = world.spawn(()).id();
        let bot = azalea::Client::new(bot_entity, Arc::new(parking_lot::Mutex::new(world)));
        assert!(collect_entities(&bot).is_empty());
    }

    /// Build a minimal live ECS world and verify `collect_entities`
    /// returns every entity in the bot's dimension with correct types,
    /// positions, ids and display names — while excluding the bot itself
    /// and entities in other dimensions.
    #[test]
    fn test_collect_entities_reads_live_ecs() {
        let overworld = azalea::world::InstanceName(azalea::Identifier::new("minecraft:overworld"));
        let nether = azalea::world::InstanceName(azalea::Identifier::new("minecraft:the_nether"));

        let mut world = bevy_ecs::world::World::new();

        // Bot entity: given full entity components below, but
        // collect_entities must still skip it (self_player covers it).
        let bot_uuid = uuid::Uuid::new_v4();
        let bot_entity = world.spawn(()).id();

        let instance = Arc::new(parking_lot::RwLock::new(azalea::world::Instance::default()));
        world.entity_mut(bot_entity).insert((
            azalea::local_player::InstanceHolder {
                instance: instance.clone(),
                partial_instance: Arc::new(parking_lot::RwLock::new(
                    azalea::world::PartialInstance::default(),
                )),
            },
            overworld.clone(),
            azalea::entity::Position::new(azalea::core::position::Vec3::new(0.5, 64.0, 0.5)),
            azalea::entity::EntityUuid::new(bot_uuid),
            azalea::entity::EntityKindComponent(EntityKind::Player),
        ));

        // Dropped item (no health, no profile).
        let item_uuid = uuid::Uuid::new_v4();
        let item_entity = world
            .spawn((
                overworld.clone(),
                azalea::entity::Position::new(azalea::core::position::Vec3::new(3.9, 64.0, -2.1)),
                azalea::entity::EntityUuid::new(item_uuid),
                azalea::entity::EntityKindComponent(EntityKind::Item),
            ))
            .id();

        // Zombie with health.
        let zombie_uuid = uuid::Uuid::new_v4();
        let zombie_entity = world
            .spawn((
                overworld.clone(),
                azalea::entity::Position::new(azalea::core::position::Vec3::new(-1.0, 70.5, 8.0)),
                azalea::entity::EntityUuid::new(zombie_uuid),
                azalea::entity::EntityKindComponent(EntityKind::Zombie),
                azalea::entity::metadata::Health(14.0),
            ))
            .id();

        // Another player with a game profile (the username source).
        let player_uuid = uuid::Uuid::new_v4();
        let _player_entity = world
            .spawn((
                overworld.clone(),
                azalea::entity::Position::new(azalea::core::position::Vec3::new(10.0, 64.0, 10.0)),
                azalea::entity::EntityUuid::new(player_uuid),
                azalea::entity::EntityKindComponent(EntityKind::Player),
                azalea::player::GameProfileComponent(azalea::auth::game_profile::GameProfile::new(
                    player_uuid,
                    "Steve".to_string(),
                )),
                azalea::entity::metadata::Health(20.0),
            ))
            .id();

        // Entity in another dimension — must be excluded.
        let _nether_entity = world.spawn((
            nether.clone(),
            azalea::entity::Position::new(azalea::core::position::Vec3::new(1.0, 64.0, 1.0)),
            azalea::entity::EntityUuid::new(uuid::Uuid::new_v4()),
            azalea::entity::EntityKindComponent(EntityKind::Zombie),
        ));

        // Minecraft entity ids as maintained by azalea's index.
        instance
            .write()
            .entity_by_id
            .insert(azalea::world::MinecraftEntityId(7), item_entity);
        instance
            .write()
            .entity_by_id
            .insert(azalea::world::MinecraftEntityId(8), zombie_entity);

        let bot = azalea::Client::new(bot_entity, Arc::new(parking_lot::Mutex::new(world)));
        let entities = collect_entities(&bot);

        assert_eq!(
            entities.len(),
            3,
            "item + zombie + player, but not the bot itself or the nether entity"
        );
        assert!(
            entities.iter().all(|e| e.uuid != bot_uuid.to_string()),
            "the bot's own entity must not appear in the entity list"
        );

        let item = entities.iter().find(|e| e.entity_type == "item").unwrap();
        assert_eq!(item.id, 7);
        assert_eq!(item.uuid, item_uuid.to_string());
        assert_eq!(item.position, BlockPos::new(3, 64, -3)); // floor()
        assert_eq!(item.display_name, None);
        assert_eq!(item.health, None);

        let zombie = entities.iter().find(|e| e.entity_type == "zombie").unwrap();
        assert_eq!(zombie.id, 8);
        assert_eq!(zombie.uuid, zombie_uuid.to_string());
        assert_eq!(zombie.position, BlockPos::new(-1, 70, 8));
        assert_eq!(zombie.health, Some(14.0));

        let player = entities.iter().find(|e| e.entity_type == "player").unwrap();
        assert_eq!(player.display_name, Some("Steve".to_string()));
        assert_eq!(player.health, Some(20.0));
        assert_eq!(
            player.id, UNKNOWN_ENTITY_ID,
            "not in entity_by_id → the id must be the explicit unknown sentinel, not 0"
        );
    }

    /// A dead mob (health metadata 0.0) must be excluded from the entity list
    /// (Bug #8) — a killed chicken lingered in the ECS until the server's
    /// RemoveEntities packet was processed, so `get_nearby_entities` reported
    /// a corpse with `health: 0.0`. Live mobs and health-less item entities
    /// must still be listed.
    #[test]
    fn test_collect_entities_skips_dead_entities() {
        let overworld = azalea::world::InstanceName(azalea::Identifier::new("minecraft:overworld"));

        let mut world = bevy_ecs::world::World::new();
        let bot_entity = world.spawn(()).id();
        let instance = Arc::new(parking_lot::RwLock::new(azalea::world::Instance::default()));
        world.entity_mut(bot_entity).insert((
            azalea::local_player::InstanceHolder {
                instance: instance.clone(),
                partial_instance: Arc::new(parking_lot::RwLock::new(
                    azalea::world::PartialInstance::default(),
                )),
            },
            overworld.clone(),
            azalea::entity::Position::new(azalea::core::position::Vec3::new(0.5, 64.0, 0.5)),
            azalea::entity::EntityUuid::new(uuid::Uuid::new_v4()),
            azalea::entity::EntityKindComponent(EntityKind::Player),
        ));

        // A live chicken — must be listed.
        let live_entity = world
            .spawn((
                overworld.clone(),
                azalea::entity::Position::new(azalea::core::position::Vec3::new(2.0, 64.0, 2.0)),
                azalea::entity::EntityUuid::new(uuid::Uuid::new_v4()),
                azalea::entity::EntityKindComponent(EntityKind::Chicken),
                azalea::entity::metadata::Health(4.0),
            ))
            .id();

        // The dead chicken (Bug #8) — health 0.0, must be skipped.
        let _dead_entity = world.spawn((
            overworld.clone(),
            azalea::entity::Position::new(azalea::core::position::Vec3::new(3.0, 64.0, 3.0)),
            azalea::entity::EntityUuid::new(uuid::Uuid::new_v4()),
            azalea::entity::EntityKindComponent(EntityKind::Chicken),
            azalea::entity::metadata::Health(0.0),
        ));

        // An item drop — no Health component, must still be listed.
        let _item_entity = world.spawn((
            overworld.clone(),
            azalea::entity::Position::new(azalea::core::position::Vec3::new(4.0, 64.0, 4.0)),
            azalea::entity::EntityUuid::new(uuid::Uuid::new_v4()),
            azalea::entity::EntityKindComponent(EntityKind::Item),
        ));

        instance
            .write()
            .entity_by_id
            .insert(azalea::world::MinecraftEntityId(1), live_entity);

        let bot = azalea::Client::new(bot_entity, Arc::new(parking_lot::Mutex::new(world)));
        let entities = collect_entities(&bot);

        assert!(
            entities.iter().all(|e| e.health != Some(0.0)),
            "dead entities must be filtered out, got: {entities:?}"
        );
        assert_eq!(
            entities
                .iter()
                .filter(|e| e.entity_type == "chicken")
                .count(),
            1,
            "only the live chicken may be listed"
        );
        assert!(
            entities.iter().any(|e| e.entity_type == "item"),
            "item entities (no Health) must be unaffected by the corpse filter"
        );
    }
}
