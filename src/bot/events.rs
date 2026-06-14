//! Event processing from the Minecraft client (chat, move, damage, etc.).

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use azalea::{Client, Event};
use azalea::ecs::component::Component;
use tracing::{debug, trace, warn};

use crate::channel::BotCommandReceiver;
use crate::snapshot::{DirtyTracker, SnapshotBuilder};
use crate::state::SharedState;
use crate::types::{BlockEntry, BlockPos, EntityEntry, GameMode, SelfPlayer};

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
    /// Receiver for bot commands sent from the MCP server.
    pub command_receiver: Arc<tokio::sync::Mutex<BotCommandReceiver>>,
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
        let (_, receiver) = crate::channel::create_command_channel(1);
        Self {
            shared_state: Arc::new(SharedState::new(crate::config::AppConfig::default())),
            command_receiver: Arc::new(tokio::sync::Mutex::new(receiver)),
            egui_ctx: None,
            dirty_tracker: Arc::new(Mutex::new(DirtyTracker::new())),
            last_snapshot_time: Arc::new(Mutex::new(
                Instant::now() - Duration::from_secs(3600),
            )),
            snapshot_interval_ms: 500,
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
            handle_spawn(&state);
        }
        Event::Disconnect(_) => {
            handle_disconnect(&state);
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
            handle_add_player(&state, &info);
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

fn handle_spawn(state: &BotState) {
    state.shared_state.set_online(true);
    request_repaint(state);
    trace!("bot spawned, set online=true");
}

fn handle_disconnect(state: &BotState) {
    state.shared_state.set_online(false);
    request_repaint(state);
    trace!("bot disconnected, set online=false");
}

async fn handle_tick(bot: Client, state: BotState) {
    let should_update = {
        let last = state
            .last_snapshot_time
            .lock()
            .expect("last_snapshot_time poisoned");
        last.elapsed() >= Duration::from_millis(state.snapshot_interval_ms)
    };
    if should_update {
        *state
            .last_snapshot_time
            .lock()
            .expect("last_snapshot_time poisoned") = Instant::now();
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

fn handle_add_player(state: &BotState, info: &azalea::player::PlayerInfo) {
    let mut snapshot = (*state.shared_state.read_snapshot()).clone();
    snapshot.entities.retain(|e| e.uuid != info.uuid.to_string());
    snapshot.entities.push(EntityEntry {
        id: 0,
        uuid: info.uuid.to_string(),
        entity_type: "player".to_string(),
        position: BlockPos::new(0, 0, 0),
        display_name: info.display_name.as_ref().map(|dt| dt.to_string()),
        health: None,
    });
    state.shared_state.update_snapshot(snapshot);
    trace!("player added: {}", info.profile.name);
}

fn handle_remove_player(state: &BotState, info: &azalea::player::PlayerInfo) {
    let mut snapshot = (*state.shared_state.read_snapshot()).clone();
    snapshot.entities.retain(|e| e.uuid != info.uuid.to_string());
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
        held_item_slot: 0,
    };

    let old_snapshot = state.shared_state.read_snapshot();

    // Take dirty sets and read world for changed blocks.
    let mut tracker = state.dirty_tracker.lock().expect("dirty_tracker poisoned");
    let (dirty_blocks, dirty_chunks) = tracker.take_dirty_sets();

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
    let chunk_summary = if let Some(world_holder) =
        bot.get_component::<azalea::local_player::WorldHolder>()
    {
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

    #[test]
    fn test_spawn_sets_online() {
        let state = BotState::default();
        handle_spawn(&state);
        assert!(state.shared_state.is_online());
    }

    #[test]
    fn test_disconnect_sets_offline() {
        let state = BotState::default();
        state.shared_state.set_online(true);
        handle_disconnect(&state);
        assert!(!state.shared_state.is_online());
    }

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
                properties: std::sync::Arc::new(azalea::auth::game_profile::GameProfileProperties::default()),
            },
            uuid: uuid::Uuid::new_v4(),
            gamemode: azalea::core::game_type::GameMode::Survival,
            latency: 20,
            display_name: Some(Box::new(azalea::FormattedText::from("SteveAdmin"))),
        };
        handle_add_player(&state, &info);
        let snapshot = state.shared_state.read_snapshot();
        assert_eq!(snapshot.entities.len(), 1);
        assert_eq!(snapshot.entities[0].uuid, info.uuid.to_string());
    }

    #[test]
    fn test_remove_player_updates_snapshot() {
        let state = BotState::default();
        let info = azalea::player::PlayerInfo {
            profile: azalea::auth::game_profile::GameProfile {
                uuid: uuid::Uuid::new_v4(),
                name: "Steve".to_string(),
                properties: std::sync::Arc::new(azalea::auth::game_profile::GameProfileProperties::default()),
            },
            uuid: uuid::Uuid::new_v4(),
            gamemode: azalea::core::game_type::GameMode::Survival,
            latency: 20,
            display_name: None,
        };
        handle_add_player(&state, &info);
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
                properties: std::sync::Arc::new(azalea::auth::game_profile::GameProfileProperties::default()),
            },
            uuid,
            gamemode: azalea::core::game_type::GameMode::Survival,
            latency: 20,
            display_name: None,
        };
        handle_add_player(&state, &info_add);

        let info_update = azalea::player::PlayerInfo {
            profile: azalea::auth::game_profile::GameProfile {
                uuid: uuid::Uuid::new_v4(),
                name: "Steve".to_string(),
                properties: std::sync::Arc::new(azalea::auth::game_profile::GameProfileProperties::default()),
            },
            uuid,
            gamemode: azalea::core::game_type::GameMode::Survival,
            latency: 20,
            display_name: Some(Box::new(azalea::FormattedText::from("SteveAdmin"))),
        };
        handle_update_player(&state, &info_update);

        let snapshot = state.shared_state.read_snapshot();
        assert_eq!(snapshot.entities[0].display_name, Some("SteveAdmin".to_string()));
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
