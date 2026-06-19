//! Event processing from the Minecraft client (chat, move, damage, etc.).

use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use azalea::ecs::component::Component;
use azalea::{Client, Event};
use tracing::{debug, info, trace, warn};

use super::commands::{CommandExecutor, RealBotClient};
use crate::channel::{ReceiverLease, ReceiverSlot};
use crate::snapshot::{DirtyTracker, SnapshotBuilder};
use crate::state::SharedState;
use crate::types::{BlockEntry, BlockPos, EntityEntry, GameMode, SelfPlayer};

// ---------------------------------------------------------------------------
// Dependency injection — set before ClientBuilder::start()
// ---------------------------------------------------------------------------

/// Pre-initialized shared state to inject into [`BotState`] before the bot
/// starts. Set by [`crate::bot::connection::ConnectionManager::connect`].
///
/// If not set, [`BotState::default`] falls back to creating an isolated
/// [`SharedState`] (useful for unit tests).
pub(crate) static INJECTED_SHARED_STATE: OnceLock<Arc<SharedState>> = OnceLock::new();

/// Pre-initialized command receiver slot to inject into [`BotState`].
///
/// The receiver is stored behind `Mutex<Option<_>>` so the event handler can
/// [`ReceiverLease::take`] it on `Event::Spawn` and the command executor can
/// run with it; when the executor is aborted the lease returns the receiver
/// to this slot, allowing a future `Spawn` (reconnect) to re-acquire it.
/// Set by [`crate::bot::connection::ConnectionManager::connect`].
pub(crate) static INJECTED_COMMAND_RECEIVER: OnceLock<ReceiverSlot> = OnceLock::new();

/// Pre-initialized egui context to inject into [`BotState`] (optional).
pub(crate) static INJECTED_EGUI_CTX: OnceLock<Option<egui::Context>> = OnceLock::new();

/// Pre-initialized snapshot interval to inject into [`BotState`].
pub(crate) static INJECTED_SNAPSHOT_INTERVAL_MS: OnceLock<u64> = OnceLock::new();

// ---------------------------------------------------------------------------
// BotState
// ---------------------------------------------------------------------------

/// State carried by the azalea event handler.
///
/// Must implement [`Clone`] + [`Component`] + [`Default`] because azalea
/// requires the state to be an ECS component and clones it for each handler
/// invocation.
#[derive(Clone, Component)]
pub struct BotState {
    /// Shared application state — updated by the handler, read by MCP and UI.
    pub shared_state: Arc<SharedState>,
    /// Slot holding the command receiver, leased out to the command executor
    /// on `Event::Spawn`. See [`ReceiverLease`].
    pub command_receiver: ReceiverSlot,
    /// Handle to the running command executor task (if any). Aborted on
    /// disconnect so the stale azalea `Client` is never used after the
    /// connection drops; the leased receiver is returned to
    /// [`BotState::command_receiver`] by the [`ReceiverLease`] drop guard.
    pub executor_handle: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    /// Optional egui context for requesting UI repaints.
    pub egui_ctx: Option<egui::Context>,
    /// Tracks which blocks/chunks changed since the last snapshot.
    pub dirty_tracker: Arc<Mutex<DirtyTracker>>,
    /// Last time a snapshot was written to [`SharedState`].
    pub last_snapshot_time: Arc<Mutex<Instant>>,
    /// Minimum milliseconds between snapshot updates.
    pub snapshot_interval_ms: u64,
}

impl Default for BotState {
    fn default() -> Self {
        let shared_state = INJECTED_SHARED_STATE
            .get()
            .cloned()
            .unwrap_or_else(|| Arc::new(SharedState::new(crate::config::AppConfig::default())));

        let command_receiver = INJECTED_COMMAND_RECEIVER.get().cloned().unwrap_or_else(|| {
            let (_, receiver) = crate::channel::create_command_channel(1);
            Arc::new(Mutex::new(Some(receiver)))
        });

        let egui_ctx = INJECTED_EGUI_CTX.get().cloned().flatten();

        let snapshot_interval_ms = INJECTED_SNAPSHOT_INTERVAL_MS.get().copied().unwrap_or(500);

        Self {
            shared_state,
            command_receiver,
            executor_handle: Arc::new(Mutex::new(None)),
            egui_ctx,
            dirty_tracker: Arc::new(Mutex::new(DirtyTracker::new())),
            last_snapshot_time: Arc::new(Mutex::new(Instant::now() - Duration::from_secs(3600))),
            snapshot_interval_ms,
        }
    }
}

// ---------------------------------------------------------------------------
// handle_event
// ---------------------------------------------------------------------------

/// Main azalea event handler.
///
/// This is a function pointer (no closures) so azalea can call it from the ECS.
/// Heavy work is offloaded via [`tokio::task::spawn_local`] where appropriate.
pub async fn handle_event(bot: Client, event: Event, state: BotState) -> eyre::Result<()> {
    match event {
        Event::Spawn => {
            handle_spawn(bot, &state);
        }
        Event::Disconnect(_) => {
            handle_disconnect(bot, &state);
        }
        Event::Tick => {
            handle_tick(bot, state).await;
        }
        Event::Chat(chat_packet) => {
            handle_chat(&state, chat_packet);
        }
        Event::Death(_) => {
            handle_death(&state);
        }
        Event::AddPlayer(info) => {
            handle_add_player(&bot, &state, &info);
        }
        Event::RemovePlayer(info) => {
            handle_remove_player(&state, &info);
        }
        Event::UpdatePlayer(info) => {
            handle_update_player(&state, &info);
        }
        Event::ReceiveChunk(chunk_pos) => {
            handle_receive_chunk(&state, chunk_pos);
        }
        _ => {}
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Event helpers
// ---------------------------------------------------------------------------

fn handle_spawn(bot: Client, state: &BotState) {
    state.shared_state.set_online(true);

    // Abort any previous command executor (e.g. left over from a prior
    // connection that dropped without firing Disconnect). Aborting drops the
    // ReceiverLease, which returns the receiver to the slot below.
    {
        let mut handle_guard = state
            .executor_handle
            .lock()
            .expect("executor_handle mutex poisoned");
        if let Some(handle) = handle_guard.take() {
            handle.abort();
            info!("aborted previous command executor before starting a new one");
        }
    }

    // Lease the command receiver and start a new executor driving it.
    match ReceiverLease::take(&state.command_receiver) {
        Some(lease) => {
            let shared_state = Arc::clone(&state.shared_state);
            let client = RealBotClient::new(bot, Arc::clone(&shared_state));
            let handle = tokio::task::spawn_local(async move {
                let mut executor = CommandExecutor::new_for_lease(client, shared_state);
                executor.run_with_lease(lease).await;
            });
            *state
                .executor_handle
                .lock()
                .expect("executor_handle mutex poisoned") = Some(handle);
            info!("command executor started");
        }
        None => {
            warn!(
                "Spawn fired but no command receiver was available — executor \
                 not started (this is expected if a previous executor is still \
                 shutting down)"
            );
        }
    }

    request_repaint(state);
    trace!("bot spawned, set online=true");
}

fn handle_disconnect(bot: Client, state: &BotState) {
    state.shared_state.set_online(false);

    // Abort the command executor so it can't use the now-stale azalea Client
    // (which would panic when touching the ECS after disconnect). The
    // ReceiverLease guard drops and returns the receiver to the slot, ready
    // for the next Spawn.
    let aborted = {
        let mut handle_guard = state
            .executor_handle
            .lock()
            .expect("executor_handle mutex poisoned");
        handle_guard.take().is_some()
    };
    if aborted {
        info!("aborted command executor on disconnect");
    }

    // Tell azalea to end the client so ClientBuilder::start returns and the
    // connection loop can retry. Without this the bot thread may hang waiting
    // for an ECS that's already shutting down.
    bot.exit();

    request_repaint(state);
    trace!("bot disconnected, set online=false");
}

async fn handle_tick(bot: Client, state: BotState) {
    // Check-and-set under a single lock to avoid the TOCTOU race where two
    // concurrent Tick events both pass the interval check before either
    // resets the timer (which would spawn two snapshot builders).
    let should_update = {
        let mut last = state
            .last_snapshot_time
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if last.elapsed() >= Duration::from_millis(state.snapshot_interval_ms) {
            *last = Instant::now();
            true
        } else {
            false
        }
    };
    if should_update {
        tokio::task::spawn_local(async move {
            if let Err(e) = build_and_update_snapshot(&bot, &state).await {
                warn!("snapshot update failed: {e}");
            }
        });
    }
}

fn handle_chat(state: &BotState, chat_packet: azalea::chat::ChatPacket) {
    let (sender, message) = chat_packet.split_sender_and_content();
    let sender = sender.unwrap_or_else(|| "System".to_string());
    state.shared_state.add_chat_message(sender, message);
    trace!("chat message stored");
}

fn handle_death(state: &BotState) {
    let mut snapshot = (*state.shared_state.read_snapshot()).clone();
    snapshot.self_player.health = 0.0;
    state.shared_state.update_snapshot(snapshot);
    request_repaint(state);
    trace!("bot died, set health=0");
}

fn handle_add_player(bot: &Client, state: &BotState, info: &azalea::player::PlayerInfo) {
    // The tab-list event fires when a player joins the server, which may be
    // before their entity has spawned in the client world. Try to read the
    // live position and minecraft entity id; fall back to defaults if the
    // entity isn't available yet (a later Tick snapshot will refresh them).
    let (id, position) = bot
        .entity_id_by_uuid(info.uuid)
        .map(|entity| {
            let position = bot
                .get_entity_component::<azalea::entity::Position>(entity)
                .map(|p| BlockPos::new(p.x as i32, p.y as i32, p.z as i32))
                .unwrap_or(BlockPos::new(0, 0, 0));
            let id = bot
                .get_entity_component::<azalea::core::entity_id::MinecraftEntityId>(entity)
                .map(|m| m.0 as u32)
                .unwrap_or(0);
            (id, position)
        })
        .unwrap_or((0, BlockPos::new(0, 0, 0)));

    add_player_to_snapshot(state, info, id, position);
}

/// Pure snapshot update for an added player — split out so it can be tested
/// without an azalea [`Client`].
fn add_player_to_snapshot(
    state: &BotState,
    info: &azalea::player::PlayerInfo,
    id: u32,
    position: BlockPos,
) {
    let mut snapshot = (*state.shared_state.read_snapshot()).clone();
    snapshot
        .entities
        .retain(|e| e.uuid != info.uuid.to_string());
    snapshot.entities.push(EntityEntry {
        id,
        uuid: info.uuid.to_string(),
        entity_type: "player".to_string(),
        position,
        display_name: info.display_name.as_ref().map(|dt| dt.to_string()),
        health: None,
    });
    state.shared_state.update_snapshot(snapshot);
    trace!("player added: {}", info.profile.name);
}

fn handle_remove_player(state: &BotState, info: &azalea::player::PlayerInfo) {
    let mut snapshot = (*state.shared_state.read_snapshot()).clone();
    snapshot
        .entities
        .retain(|e| e.uuid != info.uuid.to_string());
    state.shared_state.update_snapshot(snapshot);
    trace!("player removed: {}", info.profile.name);
}

fn handle_update_player(state: &BotState, info: &azalea::player::PlayerInfo) {
    let mut snapshot = (*state.shared_state.read_snapshot()).clone();
    if let Some(entity) = snapshot
        .entities
        .iter_mut()
        .find(|e| e.uuid == info.uuid.to_string())
    {
        entity.display_name = info.display_name.as_ref().map(|dt| dt.to_string());
    }
    state.shared_state.update_snapshot(snapshot);
    trace!("player updated: {}", info.profile.name);
}

fn handle_receive_chunk(state: &BotState, chunk_pos: azalea::core::position::ChunkPos) {
    let mut tracker = state.dirty_tracker.lock().expect("dirty_tracker poisoned");
    tracker.mark_chunk_dirty((chunk_pos.x, chunk_pos.z));
    trace!("chunk dirty marked: ({}, {})", chunk_pos.x, chunk_pos.z);
}

fn request_repaint(state: &BotState) {
    if let Some(ctx) = &state.egui_ctx {
        ctx.request_repaint();
    }
}

// ---------------------------------------------------------------------------
// Snapshot builder
// ---------------------------------------------------------------------------

async fn build_and_update_snapshot(bot: &Client, state: &BotState) -> eyre::Result<()> {
    let position = bot.component::<azalea::entity::Position>();
    let health = bot.component::<azalea::entity::metadata::Health>();
    let hunger = bot.hunger();
    let _experience = bot.experience();
    let local_gamemode = bot.component::<azalea::local_player::LocalGameMode>();
    let profile = bot.profile();

    let self_player = SelfPlayer {
        uuid: profile.uuid.to_string(),
        username: profile.name,
        position: BlockPos::new(position.x as i32, position.y as i32, position.z as i32),
        health: health.0,
        hunger: hunger.food as i32,
        gamemode: azalea_gamemode_to_ours(local_gamemode.current),
        held_item_slot: bot.selected_hotbar_slot(),
    };

    let old_snapshot = state.shared_state.read_snapshot();

    // Take dirty sets and release the tracker immediately so concurrent
    // `handle_receive_chunk` calls aren't blocked while we read the world.
    let (dirty_blocks, dirty_chunks) = {
        let mut tracker = state
            .dirty_tracker
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        tracker.take_dirty_sets()
    };

    let mut new_blocks = Vec::new();
    if !dirty_blocks.is_empty() || !dirty_chunks.is_empty() {
        let world = bot.world();
        let world_guard = world.read();
        for pos in &dirty_blocks {
            let azalea_pos = azalea::core::position::BlockPos::new(pos.x, pos.y, pos.z);
            if let Some(block_state) = world_guard.get_block_state(azalea_pos) {
                let block_name = block_state_to_name(block_state);
                new_blocks.push(BlockEntry {
                    position: *pos,
                    block_type: block_name,
                    block_state: None,
                });
            }
        }
        // Dirty chunks are tracked in the chunk summary; scanning all their
        // blocks is too expensive for a tick handler.
        drop(world_guard);
    }

    // Re-populate a temporary tracker for SnapshotBuilder.
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

    // Chunk summary from the partial world.
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

    let new_snapshot = builder.build();
    state.shared_state.update_snapshot(new_snapshot);
    request_repaint(state);
    debug!("snapshot updated");

    Ok(())
}

fn block_state_to_name(block_state: azalea::block::BlockState) -> String {
    // BlockState is a numeric ID; Block (alias for BlockKind) maps that to
    // the block type. We use the deprecated `Block` alias because `BlockKind`
    // itself is private in `azalea_registry::builtin`.
    #[allow(deprecated)]
    let block_kind = azalea::registry::Block::from(block_state);
    // Block derives Debug with the variant name (e.g. Stone).
    // We convert to snake_case to match Minecraft IDs.
    let debug_name = format!("{:?}", block_kind);
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- BotState construction -----------------------------------------------

    #[test]
    fn test_bot_state_default() {
        let state = BotState::default();
        assert!(!state.shared_state.is_online());
        assert_eq!(state.snapshot_interval_ms, 500);
        assert!(state.egui_ctx.is_none());
    }

    #[test]
    fn test_bot_state_clone_shares_arc() {
        let state = BotState::default();
        let cloned = state.clone();
        assert!(Arc::ptr_eq(&state.shared_state, &cloned.shared_state));
        assert!(Arc::ptr_eq(&state.dirty_tracker, &cloned.dirty_tracker));
    }

    // -- Event helpers (no Client needed) ------------------------------------

    // NOTE: `handle_spawn` and `handle_disconnect` now require an azalea
    // `Client` (they start/stop the command executor and call `bot.exit()`),
    // so they cannot be exercised in isolation here. Their online-flag
    // behaviour is covered by the `SharedState` tests in `state.rs`, and the
    // executor wiring is covered by `bot::commands` tests.

    #[test]
    fn test_death_sets_health_to_zero() {
        let state = BotState::default();
        handle_death(&state);
        let snapshot = state.shared_state.read_snapshot();
        assert_eq!(snapshot.self_player.health, 0.0);
    }

    #[test]
    fn test_receive_chunk_marks_dirty() {
        let state = BotState::default();
        let chunk_pos = azalea::core::position::ChunkPos::new(3, -7);
        handle_receive_chunk(&state, chunk_pos);
        let tracker = state.dirty_tracker.lock().unwrap();
        assert!(!tracker.is_empty());
    }

    // -- Chat handling -------------------------------------------------------

    #[test]
    fn test_chat_system_message() {
        let state = BotState::default();
        let chat = azalea::chat::ChatPacket::new("Hello world");
        handle_chat(&state, chat);
        let messages = state.shared_state.get_chat_messages();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].0, "System");
        assert_eq!(messages[0].1, "Hello world");
    }

    // -- Player list ---------------------------------------------------------

    #[test]
    fn test_add_player_updates_snapshot() {
        let state = BotState::default();
        let info = azalea::player::PlayerInfo {
            profile: azalea::auth::game_profile::GameProfile {
                uuid: uuid::Uuid::new_v4(),
                name: "Steve".to_string(),
                properties: std::sync::Arc::new(
                    azalea::auth::game_profile::GameProfileProperties::default(),
                ),
            },
            uuid: uuid::Uuid::new_v4(),
            gamemode: azalea::core::game_type::GameMode::Survival,
            latency: 20,
            display_name: Some(Box::new(azalea::FormattedText::from("SteveAdmin"))),
        };
        // Use the pure helper so the test doesn't need a live azalea Client.
        add_player_to_snapshot(&state, &info, 7, BlockPos::new(10, 64, -5));
        let snapshot = state.shared_state.read_snapshot();
        assert_eq!(snapshot.entities.len(), 1);
        assert_eq!(snapshot.entities[0].uuid, info.uuid.to_string());
        assert_eq!(snapshot.entities[0].id, 7);
        assert_eq!(snapshot.entities[0].position, BlockPos::new(10, 64, -5));
    }

    #[test]
    fn test_remove_player_updates_snapshot() {
        let state = BotState::default();
        let info = azalea::player::PlayerInfo {
            profile: azalea::auth::game_profile::GameProfile {
                uuid: uuid::Uuid::new_v4(),
                name: "Steve".to_string(),
                properties: std::sync::Arc::new(
                    azalea::auth::game_profile::GameProfileProperties::default(),
                ),
            },
            uuid: uuid::Uuid::new_v4(),
            gamemode: azalea::core::game_type::GameMode::Survival,
            latency: 20,
            display_name: None,
        };
        add_player_to_snapshot(&state, &info, 0, BlockPos::new(0, 0, 0));
        handle_remove_player(&state, &info);
        let snapshot = state.shared_state.read_snapshot();
        assert!(snapshot.entities.is_empty());
    }

    #[test]
    fn test_update_player_updates_snapshot() {
        let state = BotState::default();
        let uuid = uuid::Uuid::new_v4();
        let info_add = azalea::player::PlayerInfo {
            profile: azalea::auth::game_profile::GameProfile {
                uuid: uuid::Uuid::new_v4(),
                name: "Steve".to_string(),
                properties: std::sync::Arc::new(
                    azalea::auth::game_profile::GameProfileProperties::default(),
                ),
            },
            uuid,
            gamemode: azalea::core::game_type::GameMode::Survival,
            latency: 20,
            display_name: None,
        };
        add_player_to_snapshot(&state, &info_add, 0, BlockPos::new(0, 0, 0));

        let info_update = azalea::player::PlayerInfo {
            profile: azalea::auth::game_profile::GameProfile {
                uuid: uuid::Uuid::new_v4(),
                name: "Steve".to_string(),
                properties: std::sync::Arc::new(
                    azalea::auth::game_profile::GameProfileProperties::default(),
                ),
            },
            uuid,
            gamemode: azalea::core::game_type::GameMode::Survival,
            latency: 20,
            display_name: Some(Box::new(azalea::FormattedText::from("SteveAdmin"))),
        };
        handle_update_player(&state, &info_update);

        let snapshot = state.shared_state.read_snapshot();
        assert_eq!(
            snapshot.entities[0].display_name,
            Some("SteveAdmin".to_string())
        );
    }

    // -- Throttle logic ------------------------------------------------------

    #[test]
    fn test_tick_throttle_skips_fast_updates() {
        let state = BotState::default();
        state.shared_state.set_online(true);

        // Manually set last snapshot time to now.
        *state.last_snapshot_time.lock().unwrap() = Instant::now();

        // Should not update because interval hasn't passed.
        let should_update = {
            let last = state.last_snapshot_time.lock().unwrap();
            last.elapsed() >= Duration::from_millis(state.snapshot_interval_ms)
        };
        assert!(!should_update);
    }

    #[test]
    fn test_tick_throttle_allows_slow_updates() {
        let state = BotState::default();
        state.shared_state.set_online(true);

        // Set last snapshot time far in the past.
        *state.last_snapshot_time.lock().unwrap() = Instant::now() - Duration::from_secs(10);

        let should_update = {
            let last = state.last_snapshot_time.lock().unwrap();
            last.elapsed() >= Duration::from_millis(state.snapshot_interval_ms)
        };
        assert!(should_update);
    }

    // -- Utility -------------------------------------------------------------

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
    }
}
