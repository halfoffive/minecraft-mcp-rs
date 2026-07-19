//! Periodic snapshot updates driven by bot Tick events.
//!
//! [`SnapshotUpdater`] encapsulates the throttled world-state collection
//! logic: reading bot position/health/gamemode, scanning dirty blocks,
//! and atomically updating [`SharedState`] via [`WorldSnapshot`].

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use azalea::Client;
use tracing::{debug, warn};

use crate::bot::commands::item_kind_to_id;
use crate::snapshot::{DirtyTracker, SnapshotBuilder};
use crate::state::SharedState;
use crate::types::{BlockEntry, BlockPos, GameMode, InventorySlot, SelfPlayer, WorldSnapshot};
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
    fn check_and_update_timer(&self) -> bool {
        let mut last = self.last_update.lock().unwrap_or_else(|e| e.into_inner());
        if last.elapsed() >= Duration::from_millis(self.interval_ms) {
            *last = Instant::now();
            true
        } else {
            false
        }
    }

    // ── Main tick handler ───────────────────────────────────

    /// Called on every Tick event.
    ///
    /// Returns `Some(snapshot)` if a new snapshot was built and stored in
    /// [`SharedState`], or `None` if the call was throttled (interval has
    /// not elapsed yet).
    pub async fn update_from_tick(&self, bot: &Client) -> Option<WorldSnapshot> {
        if !self.check_and_update_timer() {
            return None;
        }

        match build_snapshot_inner(bot, &self.shared_state, &self.dirty_tracker).await {
            Ok(snapshot) => {
                self.shared_state.update_snapshot(snapshot.clone());
                debug!("snapshot updated via SnapshotUpdater");
                Some(snapshot)
            }
            Err(e) => {
                warn!("snapshot build failed: {e}");
                None
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// Inner snapshot builder (free function — testable in isolation)
// ═══════════════════════════════════════════════════════════════

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
    // very first tick after spawn).
    let yaw: Option<f32> = bot
        .get_component::<azalea::entity::LookDirection>()
        .map(|look| look.y_rot());

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
                                if block_name != "air" {
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

    let mut builder = SnapshotBuilder::new((*old_snapshot).clone())
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

    let mut snapshot = builder.build();
    // Populate `commands_enabled` from the player's permission level.
    // OP level > 0 means commands are enabled; == 0 means disabled;
    // unavailable (component not yet present) means unknown.
    snapshot.commands_enabled = read_commands_enabled(bot);
    Ok(snapshot)
}

// ═══════════════════════════════════════════════════════════════
// Utility helpers
// ═══════════════════════════════════════════════════════════════

/// Read the player's 36-slot inventory from the azalea client.
///
/// Mirrors the logic in [`crate::bot::commands::RealBotClient::inventory_entries`]:
/// when a container is open the menu is not `Player`, so we return an empty
/// list rather than stale container slots. Only non-empty slots are returned.
fn read_inventory(bot: &Client) -> Vec<InventorySlot> {
    let menu = bot.menu();
    let player = match menu.try_as_player() {
        Some(p) => p,
        None => return Vec::new(),
    };
    player
        .inventory
        .iter()
        .enumerate()
        .filter_map(|(slot, stack)| {
            if stack.is_empty() {
                None
            } else {
                Some(InventorySlot {
                    slot_index: slot as u8,
                    item_id: item_kind_to_id(stack.kind()),
                    count: stack.count().clamp(0, 255) as u8,
                })
            }
        })
        .collect()
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

fn block_state_to_name(block_state: azalea::block::BlockState) -> String {
    #[allow(deprecated)]
    let block_kind = azalea::registry::Block::from(block_state);
    let debug_name = format!("{block_kind:?}");
    to_snake_case(&debug_name)
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
}
