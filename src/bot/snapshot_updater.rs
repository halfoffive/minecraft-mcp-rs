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
    /// All four parameters are typically extracted from [`BotState`]
    /// (see [`crate::bot::events::BotState`]) so that the updater shares
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
        if let Ok(mut tracker) = self.dirty_tracker.lock() {
            tracker.mark_block_dirty(pos);
        }
    }

    /// Mark an entire chunk as dirty so that the next snapshot re-reads
    /// all blocks in that chunk.
    pub fn mark_chunk_dirty(&self, chunk: (i32, i32)) {
        if let Ok(mut tracker) = self.dirty_tracker.lock() {
            tracker.mark_chunk_dirty(chunk);
        }
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

    let inventory = read_inventory(bot);

    let self_player = SelfPlayer {
        uuid: profile.uuid.to_string(),
        username: profile.name,
        position: BlockPos::new(position.x as i32, position.y as i32, position.z as i32),
        health: health.0,
        hunger: hunger.food as i32,
        gamemode: azalea_gamemode_to_ours(local_gamemode.current),
        held_item_slot: bot.selected_hotbar_slot(),
        inventory,
    };

    // ── Read old snapshot ────────────────────────────────────
    let old_snapshot = shared_state.read_snapshot();

    // ── Drain dirty sets ─────────────────────────────────────
    let (dirty_blocks, dirty_chunks) = {
        let mut tracker = dirty_tracker.lock().unwrap_or_else(|e| e.into_inner());
        tracker.take_dirty_sets()
    };

    // ── Read world for changed blocks ────────────────────────
    let mut new_blocks = Vec::new();
    if !dirty_blocks.is_empty() || !dirty_chunks.is_empty() {
        let world = bot.world();
        let world_guard = world.read();
        for pos in &dirty_blocks {
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
        // Full-chunk scanning is too expensive per tick; dirty chunks
        // are reflected in the chunk summary instead.
        drop(world_guard);
    }

    // ── Repopulate tracker for SnapshotBuilder ───────────────
    let mut builder_tracker = DirtyTracker::new();
    for pos in &dirty_blocks {
        builder_tracker.mark_block_dirty(*pos);
    }
    for chunk in &dirty_chunks {
        builder_tracker.mark_chunk_dirty(*chunk);
    }

    let mut builder = SnapshotBuilder::new((*old_snapshot).clone())
        .with_dirty_tracker(&mut builder_tracker)
        .with_self_player(self_player);

    if !new_blocks.is_empty() {
        builder = builder.with_blocks(new_blocks);
    }

    // ── Chunk summary from partial world ─────────────────────
    let chunk_summary =
        if let Some(world_holder) = bot.get_component::<azalea::local_player::WorldHolder>() {
            let partial_world = world_holder.partial.read();
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

    Ok(builder.build())
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

fn block_state_to_name(block_state: azalea::block::BlockState) -> String {
    #[allow(deprecated)]
    let block_kind = azalea::registry::Block::from(block_state);
    let debug_name = format!("{block_kind:?}");
    to_snake_case(&debug_name)
}

fn to_snake_case(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 4);
    for (i, ch) in s.chars().enumerate() {
        if ch.is_uppercase() {
            if i > 0 {
                result.push('_');
            }
            result.push(ch.to_ascii_lowercase());
        } else {
            result.push(ch);
        }
    }
    result
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
