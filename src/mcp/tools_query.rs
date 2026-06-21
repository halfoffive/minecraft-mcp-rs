//! MCP query tools for reading world/entity/player state from SharedState.
//!
//! Each function checks `SharedState::is_online()` first: if the bot is
//! offline, `is_connected` returns `{"connected":false}` and all other
//! query tools return `{"error":"Bot is currently offline"}`.

use std::borrow::Cow;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::state::SharedState;
use crate::types::GameMode;

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

/// Get the bot's full player inventory.
///
/// Returns the 36 main slots as an array of occupied slots (empty slots are
/// omitted), plus the currently selected hotbar slot. Data is read from the
/// latest [`WorldSnapshot`](crate::types::WorldSnapshot) written by the
/// snapshot updater.
pub fn get_inventory(state: &Arc<SharedState>) -> String {
    if !state.is_online() {
        return OFFLINE_ERROR.to_string();
    }
    let snapshot = state.read_snapshot();
    json!({
        "inventory": snapshot.self_player.inventory,
        "held_item_slot": snapshot.self_player.held_item_slot,
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
// get_server_info — reports commands_enabled and current gamemode
// ---------------------------------------------------------------------------

/// Convert a [`GameMode`] to its lowercase string name for JSON output.
fn gamemode_to_str(mode: GameMode) -> &'static str {
    match mode {
        GameMode::Survival => "survival",
        GameMode::Creative => "creative",
        GameMode::Adventure => "adventure",
        GameMode::Spectator => "spectator",
    }
}

/// Report whether commands are enabled on the server and the current gamemode.
///
/// `commands_enabled` is `true` if the player has OP level > 0, `false` if OP
/// level is 0, and `null` if unknown (the snapshot has not yet been populated
/// by a `QueryServerInfo` round-trip). `gamemode` is one of
/// `survival|creative|adventure|spectator`.
pub fn get_server_info(state: &Arc<SharedState>) -> String {
    if !state.is_online() {
        return OFFLINE_ERROR.to_string();
    }
    let snapshot = state.read_snapshot();
    json!({
        "commands_enabled": snapshot.commands_enabled,
        "gamemode": gamemode_to_str(snapshot.self_player.gamemode),
    })
    .to_string()
}

// ---------------------------------------------------------------------------
// get_world_view — top-down PNG render for multimodal models
// ---------------------------------------------------------------------------

/// Input for the `get_world_view` MCP tool.
#[derive(Deserialize, Default)]
pub struct GetWorldViewInput {
    pub radius: u8,
}

impl rmcp::schemars::JsonSchema for GetWorldViewInput {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("GetWorldViewInput")
    }

    fn json_schema(_gen: &mut rmcp::schemars::SchemaGenerator) -> rmcp::schemars::Schema {
        let map: Map<String, Value> = json!({
            "type": "object",
            "properties": {
                "radius": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 32,
                    "description": "Half-extent of the top-down view in blocks (1-32). The rendered image is (2*radius+1) x (2*radius+1) pixels."
                }
            },
            "required": ["radius"],
            "additionalProperties": false
        })
        .as_object()
        .cloned()
        .unwrap_or_default();
        rmcp::schemars::Schema::from(map)
    }
}

/// Render a top-down PNG of the world around the bot and return it as MCP
/// image content.
///
/// Validates `radius` is in `1..=32`, checks online status, renders the
/// snapshot via [`crate::mcp::render::render_topdown`], base64-encodes the
/// PNG bytes, and wraps them in `Content::Image` with `mime_type: "image/png"`.
/// On error, returns `Content::Text` carrying a JSON error string so the
/// caller still receives a meaningful payload.
pub fn get_world_view(state: &Arc<SharedState>, radius: u8) -> rmcp::model::Content {
    if !(1..=32).contains(&radius) {
        return rmcp::model::Content::text(format!(
            r#"{{"success":false,"error":"radius must be 1-32, got {radius}}}"#
        ));
    }
    if !state.is_online() {
        return rmcp::model::Content::text(OFFLINE_ERROR);
    }

    let snapshot = state.read_snapshot();
    let png_bytes = crate::mcp::render::render_topdown(&snapshot, radius);
    let encoded = crate::mcp::render::base64_encode(&png_bytes);
    rmcp::model::Content::image(encoded, "image/png")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::types::{
        BlockEntry, BlockPos, EntityEntry, GameMode, InventorySlot, SelfPlayer, WorldSnapshot,
    };
    use base64::Engine;

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
                inventory: vec![
                    InventorySlot {
                        slot_index: 0,
                        item_id: "iron_pickaxe".into(),
                        count: 1,
                    },
                    InventorySlot {
                        slot_index: 1,
                        item_id: "oak_planks".into(),
                        count: 64,
                    },
                ],
            },
            timestamp: 42,
            chunk_summary: vec![(0, 0), (-1, 0)],
            commands_enabled: None,
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
        assert!(result.contains("inventory"));
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        let inventory = parsed["inventory"].as_array().unwrap();
        assert_eq!(inventory.len(), 2);
        assert_eq!(inventory[0]["slot_index"], 0);
        assert_eq!(inventory[0]["item_id"], "iron_pickaxe");
        assert_eq!(inventory[0]["count"], 1);
        assert_eq!(inventory[1]["slot_index"], 1);
        assert_eq!(inventory[1]["item_id"], "oak_planks");
        assert_eq!(inventory[1]["count"], 64);
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

    // -- get_server_info -------------------------------------------------------

    #[test]
    fn test_get_server_info_online() {
        let state = state_with_snapshot();
        let result = get_server_info(&state);
        let parsed: serde_json::Value = serde_json::from_str(&result).expect("valid JSON");
        // The default snapshot in state_with_snapshot() does not set
        // commands_enabled, so it should be null.
        assert!(parsed["commands_enabled"].is_null());
        assert_eq!(parsed["gamemode"], "survival");
    }

    #[test]
    fn test_get_server_info_with_commands_enabled() {
        let state = SharedState::new(AppConfig::default());
        state.set_online(true);
        let snap = WorldSnapshot {
            commands_enabled: Some(true),
            self_player: SelfPlayer {
                gamemode: GameMode::Creative,
                ..Default::default()
            },
            ..Default::default()
        };
        state.update_snapshot(snap);

        let result = get_server_info(&Arc::new(state));
        let parsed: serde_json::Value = serde_json::from_str(&result).expect("valid JSON");
        assert_eq!(parsed["commands_enabled"], true);
        assert_eq!(parsed["gamemode"], "creative");
    }

    #[test]
    fn test_get_server_info_offline() {
        let state = offline_state();
        let result = get_server_info(&state);
        assert_eq!(result, OFFLINE_ERROR);
    }

    // -- get_world_view --------------------------------------------------------

    /// Verify that `Content::Image` is returned for a valid online call.
    #[test]
    fn test_get_world_view_online_returns_image() {
        let state = state_with_snapshot();
        let content = get_world_view(&state, 4);
        let raw = content.raw;
        match raw {
            rmcp::model::RawContent::Image(img) => {
                assert_eq!(img.mime_type, "image/png");
                assert!(!img.data.is_empty(), "base64 data should be non-empty");
                // The decoded bytes should start with the PNG magic header.
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(&img.data)
                    .expect("base64 decode");
                assert!(
                    decoded.starts_with(&[0x89, 0x50, 0x4E, 0x47]),
                    "decoded bytes should start with PNG magic, got: {:?}",
                    &decoded[..4.min(decoded.len())]
                );
            }
            other => panic!("expected Image content, got: {other:?}"),
        }
    }

    #[test]
    fn test_get_world_view_offline_returns_text_error() {
        let state = offline_state();
        let content = get_world_view(&state, 4);
        match content.raw {
            rmcp::model::RawContent::Text(text) => {
                assert!(text.text.contains("offline"), "got: {}", text.text);
            }
            other => panic!("expected Text content when offline, got: {other:?}"),
        }
    }

    #[test]
    fn test_get_world_view_invalid_radius_zero() {
        let state = state_with_snapshot();
        let content = get_world_view(&state, 0);
        match content.raw {
            rmcp::model::RawContent::Text(text) => {
                assert!(
                    text.text.contains("radius must be 1-32"),
                    "got: {}",
                    text.text
                );
            }
            other => panic!("expected Text error for radius=0, got: {other:?}"),
        }
    }

    #[test]
    fn test_get_world_view_invalid_radius_too_large() {
        let state = state_with_snapshot();
        let content = get_world_view(&state, 33);
        match content.raw {
            rmcp::model::RawContent::Text(text) => {
                assert!(
                    text.text.contains("radius must be 1-32"),
                    "got: {}",
                    text.text
                );
            }
            other => panic!("expected Text error for radius=33, got: {other:?}"),
        }
    }

    #[test]
    fn test_get_world_view_radius_boundaries_valid() {
        let state = state_with_snapshot();
        // radius = 1 and radius = 32 should both succeed.
        for radius in [1u8, 32] {
            let content = get_world_view(&state, radius);
            assert!(
                matches!(content.raw, rmcp::model::RawContent::Image(_)),
                "radius {radius} should produce image content"
            );
        }
    }
}
