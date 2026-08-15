//! Periodic snapshot updates driven by bot Tick events.
//!
//! [`SnapshotUpdater`] encapsulates the throttled world-state collection
//! logic: reading bot position/health/gamemode, scanning dirty blocks,
//! and atomically updating [`SharedState`] via [`WorldSnapshot`].

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use azalea::Client;
use tracing::{debug, warn};

use crate::bot::commands::canonical_player_inventory;
use crate::snapshot::{DirtyTracker, SnapshotBuilder};
use crate::state::SharedState;
use crate::types::{
    BlockEntry, BlockPos, EntityEntry, GameMode, InventorySlot, SelfPlayer, WorldSnapshot,
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

    // ── Main tick handler ───────────────────────────────────

    /// Build a fresh snapshot and store it in [`SharedState`], moving the
    /// snapshot in (no clone — the previous `update_from_tick` cloned the
    /// whole snapshot just to return it; the caller only needs a bool).
    ///
    /// Returns `true` if a new snapshot was built and stored (callers use
    /// this to trigger a UI repaint).
    ///
    /// The caller is expected to have passed the
    /// [`check_and_update_timer`](Self::check_and_update_timer) gate first
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
                false
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// Inner snapshot builder (free function — testable in isolation)
// ═══════════════════════════════════════════════════════════════

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
    let yaw: Option<f32> = bot
        .get_component::<azalea::entity::LookDirection>()
        .map(|look| crate::utils::normalize_yaw(look.y_rot()));

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
    if !dirty_blocks.is_empty() || !dirty_chunks.is_empty() {
        let world = bot.world();
        let world_guard = world.read();

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
                let block_name = block_state_to_name(block_state);
                new_blocks.push(BlockEntry {
                    position: *pos,
                    block_type: block_name,
                    block_state: None,
                });
            }
        }

        // M-7: Only scan dirty chunks within `chunk_scan_radius` of the
        // player's current chunk. Chunks outside this radius are skipped to
        // avoid expensive 98,304-position scans for far-away chunks.
        //
        // Scan each dirty chunk in full so the blocks inside it are re-read
        // from the world. SnapshotBuilder::build() removes every block whose
        // chunk is in `dirty_chunks` (within radius — see builder_tracker
        // below), so without re-adding the surviving blocks here they would
        // be permanently lost from the snapshot.
        //
        // Chunk dimensions for Minecraft 1.21 are 16×384×16: x and z span
        // 16 blocks within the chunk, y spans the full build height
        // (-64..320). Only non-air blocks are recorded to avoid bloating
        // the snapshot with empty positions.
        //
        // Section-level scan optimisation: a chunk has 24 vertical sections
        // (16×16×16 = 4096 blocks each). Before scanning a section, we read
        // its `block_count` (number of non-air blocks) from the chunk's
        // `sections` array. If `block_count == 0`, the entire section is
        // air and we skip its 4096 `get_block_state` calls — for typical
        // surface chunks this skips ~23/24 sections, reducing the per-chunk
        // scan from 98304 calls to ~4096 (only the surface section).
        for &(chunk_x, chunk_z) in &dirty_chunks {
            let dist = ((chunk_x - player_chunk.0).abs()).max((chunk_z - player_chunk.1).abs());
            if dist > chunk_scan_radius {
                continue;
            }
            let base_x = chunk_x * 16;
            let base_z = chunk_z * 16;

            // Build the list of section indices to scan. We hold the
            // chunk's read lock just long enough to read each section's
            // `block_count`, then release before the per-block scan
            // (which acquires its own lock through `world.get_block_state`).
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
                    // Chunk not loaded (shouldn't happen for a dirty chunk,
                    // but be defensive): fall back to scanning all 24
                    // sections so we don't silently drop blocks.
                    (0..24).collect()
                };

            for section_idx in sections_to_scan {
                // Section 0 covers y=-64..-48, section 1 covers y=-48..-32,
                // ..., section 23 covers y=304..320.
                let section_min_y = -64 + (section_idx as i32) * 16;
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
        drop(world_guard);
    }

    // ── Repopulate tracker for SnapshotBuilder ───────────────
    // M-7: Only mark chunks within scan radius as dirty in the builder
    // tracker. Far-away dirty chunks are not scanned, and their old blocks
    // are preserved (stale but acceptable for far-away regions). Individual
    // dirty blocks are always marked so their old versions are removed and
    // the freshly-read versions are added.
    let mut builder_tracker = DirtyTracker::new();
    for pos in &dirty_blocks {
        builder_tracker.mark_block_dirty(*pos);
    }
    for &(chunk_x, chunk_z) in &dirty_chunks {
        let dist = ((chunk_x - player_chunk.0).abs()).max((chunk_z - player_chunk.1).abs());
        if dist <= chunk_scan_radius {
            builder_tracker.mark_chunk_dirty((chunk_x, chunk_z));
        }
    }

    // Only the old snapshot's blocks are carried into the builder — the
    // production path replaces every other field via `with_*` below, so the
    // old snapshot (incl. its `block_index` HashMap) is not deep-cloned.
    // Before carrying them over, prune blocks outside `retention_chunks` so
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
        let minecraft_id = id_by_entity.get(&entity).copied().unwrap_or(0);
        entries.push(EntityEntry {
            id: u32::try_from(minecraft_id).unwrap_or(0),
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
        assert_eq!(player.id, 0, "not in entity_by_id → id falls back to 0");
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
