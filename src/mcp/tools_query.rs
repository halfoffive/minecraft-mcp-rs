//! MCP query tools for reading world/entity/player state from SharedState.
//!
//! Each function checks `SharedState::is_online()` first: if the bot is
//! offline, `is_connected` returns `{"connected":false}` and all other
//! query tools return `{"error":"Bot is currently offline"}`.

use std::sync::Arc;

use serde_json::json;

use crate::state::SharedState;

/// JSON error returned when the bot is offline and a query tool is called
/// (other than `is_connected`).
const OFFLINE_ERROR: &str = r#"{"error":"Bot is currently offline"}"#;

// ---------------------------------------------------------------------------
// Public query functions — called from the #[tool] methods in server.rs
// ---------------------------------------------------------------------------

/// Get information about the bot's own player (uuid, username, position,
/// health, hunger, gamemode, held item).
///
/// Returns the serialized [`crate::types::SelfPlayer`] as a JSON string,
/// or an offline error.
pub fn get_self_info(state: &Arc<SharedState>) -> String {
    if !state.is_online() {
        return OFFLINE_ERROR.to_string();
    }
    let snapshot = state.read_snapshot();
    serde_json::to_string(&snapshot.self_player)
        .unwrap_or_else(|e| json!({"error": format!("Serialization error: {e}")}).to_string())
}

/// Get the bot's currently held item slot.
///
/// Full inventory tracking is not yet implemented (the azalea ECS inventory
/// is rich but not yet wired into the snapshot).  This stub returns the
/// `held_item_slot` from [`crate::types::SelfPlayer`] plus a note so the
/// caller knows what to expect.
pub fn get_inventory(state: &Arc<SharedState>) -> String {
    if !state.is_online() {
        return OFFLINE_ERROR.to_string();
    }
    let snapshot = state.read_snapshot();
    json!({
        "held_item_slot": snapshot.self_player.held_item_slot,
        "note": "Full inventory tracking not yet implemented — only held_item_slot is available"
    })
    .to_string()
}

/// Get blocks near the bot within the given Chebyshev (square) radius.
///
/// If `filter_type` is `Some(ft)` and non-empty, only blocks whose
/// `block_type` contains `ft` (case-insensitive substring match) are
/// included.
pub fn get_nearby_blocks(
    state: &Arc<SharedState>,
    radius: u32,
    filter_type: Option<String>,
) -> String {
    if !state.is_online() {
        return OFFLINE_ERROR.to_string();
    }
    let snapshot = state.read_snapshot();
    let center = snapshot.self_player.position;
    let r = radius as i32;

    let blocks: Vec<&crate::types::BlockEntry> = snapshot
        .blocks
        .iter()
        .filter(|b| {
            (b.position.x - center.x).abs() <= r
                && (b.position.y - center.y).abs() <= r
                && (b.position.z - center.z).abs() <= r
        })
        .filter(|b| match &filter_type {
            Some(ft) if !ft.is_empty() => b.block_type.to_lowercase().contains(&ft.to_lowercase()),
            _ => true,
        })
        .collect();

    serde_json::to_string(&blocks)
        .unwrap_or_else(|e| json!({"error": format!("Serialization error: {e}")}).to_string())
}

/// Get entities near the bot within the given Chebyshev (square) radius.
pub fn get_nearby_entities(state: &Arc<SharedState>, radius: u32) -> String {
    if !state.is_online() {
        return OFFLINE_ERROR.to_string();
    }
    let snapshot = state.read_snapshot();
    let center = snapshot.self_player.position;
    let r = radius as i32;

    let entities: Vec<&crate::types::EntityEntry> = snapshot
        .entities
        .iter()
        .filter(|e| {
            (e.position.x - center.x).abs() <= r
                && (e.position.y - center.y).abs() <= r
                && (e.position.z - center.z).abs() <= r
        })
        .collect();

    serde_json::to_string(&entities)
        .unwrap_or_else(|e| json!({"error": format!("Serialization error: {e}")}).to_string())
}

/// Get a summary of chunks currently loaded around the bot.
///
/// Returns a JSON array of `(chunk_x, chunk_z)` tuples.
pub fn get_chunk_summary(state: &Arc<SharedState>) -> String {
    if !state.is_online() {
        return OFFLINE_ERROR.to_string();
    }
    let snapshot = state.read_snapshot();
    serde_json::to_string(&snapshot.chunk_summary)
        .unwrap_or_else(|e| json!({"error": format!("Serialization error: {e}")}).to_string())
}

/// Check whether the bot is currently connected to a Minecraft server.
///
/// Returns `{"connected":true}` or `{"connected":false}`.
pub fn is_connected(state: &Arc<SharedState>) -> String {
    json!({"connected": state.is_online()}).to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::types::{BlockEntry, BlockPos, EntityEntry, GameMode, SelfPlayer, WorldSnapshot};

    // -- Helpers ---------------------------------------------------------------

    /// Create a SharedState with a rich snapshot already loaded.
    fn state_with_snapshot() -> Arc<SharedState> {
        let state = SharedState::new(AppConfig::default());
        state.set_online(true);

        let snap = WorldSnapshot {
            blocks: vec![
                BlockEntry {
                    position: BlockPos::new(0, 64, 0),
                    block_type: "stone".into(),
                    block_state: None,
                },
                BlockEntry {
                    position: BlockPos::new(0, 65, 0),
                    block_type: "dirt".into(),
                    block_state: None,
                },
                BlockEntry {
                    position: BlockPos::new(10, 64, 0),
                    block_type: "diamond_ore".into(),
                    block_state: None,
                },
            ],
            entities: vec![
                EntityEntry {
                    id: 1,
                    uuid: "e1".into(),
                    entity_type: "zombie".into(),
                    position: BlockPos::new(1, 64, 0),
                    display_name: Some("Zombie".into()),
                    health: Some(20.0),
                },
                EntityEntry {
                    id: 2,
                    uuid: "e2".into(),
                    entity_type: "creeper".into(),
                    position: BlockPos::new(100, 64, 0),
                    display_name: None,
                    health: Some(20.0),
                },
            ],
            self_player: SelfPlayer {
                uuid: "player-uuid".into(),
                username: "TestBot".into(),
                position: BlockPos::new(0, 64, 0),
                health: 18.0,
                hunger: 15,
                gamemode: GameMode::Survival,
                held_item_slot: 3,
            },
            timestamp: 42,
            chunk_summary: vec![(0, 0), (-1, 0)],
        };
        state.update_snapshot(snap);
        Arc::new(state)
    }

    /// SharedState with the bot offline.
    fn offline_state() -> Arc<SharedState> {
        Arc::new(SharedState::new(AppConfig::default()))
    }

    // -- get_self_info ---------------------------------------------------------

    #[test]
    fn test_get_self_info_online() {
        let state = state_with_snapshot();
        let result = get_self_info(&state);
        assert!(result.contains("TestBot"));
        assert!(result.contains("player-uuid"));
        assert!(result.contains("18.0")); // health
        assert!(result.contains("15")); // hunger
    }

    #[test]
    fn test_get_self_info_offline() {
        let state = offline_state();
        let result = get_self_info(&state);
        assert_eq!(result, OFFLINE_ERROR);
    }

    // -- get_inventory ---------------------------------------------------------

    #[test]
    fn test_get_inventory_online() {
        let state = state_with_snapshot();
        let result = get_inventory(&state);
        assert!(result.contains("held_item_slot"));
        assert!(result.contains('3'));
        assert!(result.contains("Full inventory tracking not yet implemented"));
    }

    #[test]
    fn test_get_inventory_offline() {
        let state = offline_state();
        let result = get_inventory(&state);
        assert_eq!(result, OFFLINE_ERROR);
    }

    // -- get_nearby_blocks -----------------------------------------------------

    #[test]
    fn test_get_nearby_blocks_radius_1() {
        let state = state_with_snapshot();
        let result = get_nearby_blocks(&state, 1, None);
        // Within radius 1 of (0,64,0): stone at (0,64,0), dirt at (0,65,0)
        assert!(result.contains("stone"));
        assert!(result.contains("dirt"));
        // diamond_ore at (10,64,0) is too far
        assert!(!result.contains("diamond_ore"));
    }

    #[test]
    fn test_get_nearby_blocks_filter() {
        let state = state_with_snapshot();
        let result = get_nearby_blocks(&state, 1, Some("stone".into()));
        assert!(result.contains("stone"));
        assert!(!result.contains("dirt"));
    }

    #[test]
    fn test_get_nearby_blocks_empty_filter_acts_as_none() {
        let state = state_with_snapshot();
        let result = get_nearby_blocks(&state, 1, Some("".into()));
        assert!(result.contains("stone"));
        assert!(result.contains("dirt"));
    }

    #[test]
    fn test_get_nearby_blocks_offline() {
        let state = offline_state();
        let result = get_nearby_blocks(&state, 5, None);
        assert_eq!(result, OFFLINE_ERROR);
    }

    // -- get_nearby_entities ---------------------------------------------------

    #[test]
    fn test_get_nearby_entities_radius_1() {
        let state = state_with_snapshot();
        let result = get_nearby_entities(&state, 1);
        assert!(result.contains("zombie"));
        assert!(!result.contains("creeper")); // creeper at (100,64,0) is far
    }

    #[test]
    fn test_get_nearby_entities_large_radius() {
        let state = state_with_snapshot();
        let result = get_nearby_entities(&state, 200);
        assert!(result.contains("zombie"));
        assert!(result.contains("creeper"));
    }

    #[test]
    fn test_get_nearby_entities_offline() {
        let state = offline_state();
        let result = get_nearby_entities(&state, 10);
        assert_eq!(result, OFFLINE_ERROR);
    }

    // -- get_chunk_summary -----------------------------------------------------

    #[test]
    fn test_get_chunk_summary_online() {
        let state = state_with_snapshot();
        let result = get_chunk_summary(&state);
        assert!(result.contains("[0,0]"));
        assert!(result.contains("[-1,0]"));
    }

    #[test]
    fn test_get_chunk_summary_offline() {
        let state = offline_state();
        let result = get_chunk_summary(&state);
        assert_eq!(result, OFFLINE_ERROR);
    }

    // -- is_connected ----------------------------------------------------------

    #[test]
    fn test_is_connected_online() {
        let state = state_with_snapshot();
        let result = is_connected(&state);
        assert_eq!(result, r#"{"connected":true}"#);
    }

    #[test]
    fn test_is_connected_offline() {
        let state = offline_state();
        let result = is_connected(&state);
        assert_eq!(result, r#"{"connected":false}"#);
    }
}
