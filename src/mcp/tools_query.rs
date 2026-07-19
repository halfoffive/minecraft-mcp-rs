//! MCP query tools for reading world/entity/player state from SharedState.
//!
//! Each function checks `SharedState::is_online()` first: if the bot is
//! offline, `is_connected` returns `{"connected":false}` and all other
//! query tools return `{"error":"Bot is currently offline"}`.

use std::sync::Arc;

use serde::Deserialize;
use serde_json::json;

use crate::command_validate::clamp_to_i32;
use crate::error::BotError;
use crate::state::SharedState;
use crate::types::GameMode;

// ---------------------------------------------------------------------------
// Public query functions — called from the #[tool] methods in server.rs
// ---------------------------------------------------------------------------

/// Get information about the bot's own player (uuid, username, position,
/// health, hunger, gamemode, held item).
///
/// Returns the serialized [`crate::types::SelfPlayer`] as a JSON string,
/// or an offline error.
pub fn get_self_info(state: &Arc<SharedState>) -> Result<String, BotError> {
    if !state.is_online() {
        return Err(BotError::Offline("Bot is currently offline".to_string()));
    }
    let snapshot = state.read_snapshot();
    serde_json::to_string(&snapshot.self_player)
        .map_err(|e| BotError::Internal(format!("Serialization error: {e}")))
}

/// Get the bot's full player inventory.
///
/// Returns the 36 main slots as an array of occupied slots (empty slots are
/// omitted), plus the currently selected hotbar slot. Data is read from the
/// latest [`WorldSnapshot`](crate::types::WorldSnapshot) written by the
/// snapshot updater.
pub fn get_inventory(state: &Arc<SharedState>) -> Result<String, BotError> {
    if !state.is_online() {
        return Err(BotError::Offline("Bot is currently offline".to_string()));
    }
    let snapshot = state.read_snapshot();
    Ok(json!({
        "inventory": snapshot.self_player.inventory,
        "held_item_slot": snapshot.self_player.held_item_slot,
    })
    .to_string())
}

// ---------------------------------------------------------------------------
// Nearby query input structs
// ---------------------------------------------------------------------------

/// Input for the `get_nearby_blocks` MCP tool.
#[derive(Deserialize, Default, rmcp::schemars::JsonSchema)]
pub struct NearbyBlocksInput {
    /// Chebyshev (square) radius around the bot to search. Range: 1..=100.
    #[schemars(range(min = 1, max = 100))]
    pub radius: u32,
    /// Optional case-insensitive substring filter on block_type. If None or
    /// empty, all block types are returned.
    pub filter_type: Option<String>,
}

/// Input for the `get_nearby_entities` MCP tool.
#[derive(Deserialize, Default, rmcp::schemars::JsonSchema)]
pub struct NearbyEntitiesInput {
    /// Chebyshev (square) radius around the bot to search. Range: 1..=100.
    #[schemars(range(min = 1, max = 100))]
    pub radius: u32,
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
) -> Result<String, BotError> {
    if !(1..=100).contains(&radius) {
        return Err(BotError::InvalidParams(format!(
            "radius must be in range 1..=100, got {radius}"
        )));
    }
    if !state.is_online() {
        return Err(BotError::Offline("Bot is currently offline".to_string()));
    }
    let snapshot = state.read_snapshot();
    let center = snapshot.self_player.position;
    let r = clamp_to_i32(radius);

    // Pre-compute the lowercased filter once outside the hot closure so we
    // don't allocate a new `String` per block when filtering thousands of
    // nearby blocks.
    let ft_lower = filter_type
        .as_deref()
        .filter(|ft| !ft.is_empty())
        .map(|ft| ft.to_lowercase());

    let blocks: Vec<&crate::types::BlockEntry> = snapshot
        .blocks
        .iter()
        .filter(|b| {
            (b.position.x - center.x).abs() <= r
                && (b.position.y - center.y).abs() <= r
                && (b.position.z - center.z).abs() <= r
        })
        .filter(|b| match &ft_lower {
            Some(ft) => b.block_type.to_lowercase().contains(ft.as_str()),
            None => true,
        })
        .collect();

    serde_json::to_string(&blocks)
        .map_err(|e| BotError::Internal(format!("Serialization error: {e}")))
}

/// Get entities near the bot within the given Chebyshev (square) radius.
pub fn get_nearby_entities(state: &Arc<SharedState>, radius: u32) -> Result<String, BotError> {
    if !(1..=100).contains(&radius) {
        return Err(BotError::InvalidParams(format!(
            "radius must be in range 1..=100, got {radius}"
        )));
    }
    if !state.is_online() {
        return Err(BotError::Offline("Bot is currently offline".to_string()));
    }
    let snapshot = state.read_snapshot();
    let center = snapshot.self_player.position;
    let r = clamp_to_i32(radius);

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
        .map_err(|e| BotError::Internal(format!("Serialization error: {e}")))
}

/// Get a summary of chunks currently loaded around the bot.
///
/// Returns a JSON array of `(chunk_x, chunk_z)` tuples.
pub fn get_chunk_summary(state: &Arc<SharedState>) -> Result<String, BotError> {
    if !state.is_online() {
        return Err(BotError::Offline("Bot is currently offline".to_string()));
    }
    let snapshot = state.read_snapshot();
    serde_json::to_string(&snapshot.chunk_summary)
        .map_err(|e| BotError::Internal(format!("Serialization error: {e}")))
}

/// Check whether the bot is currently connected to a Minecraft server.
///
/// Returns `{"connected":true}` or `{"connected":false}`.
pub fn is_connected(state: &Arc<SharedState>) -> Result<String, BotError> {
    Ok(json!({"connected": state.is_online()}).to_string())
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
pub fn get_server_info(state: &Arc<SharedState>) -> Result<String, BotError> {
    if !state.is_online() {
        return Err(BotError::Offline("Bot is currently offline".to_string()));
    }
    let snapshot = state.read_snapshot();
    Ok(json!({
        "commands_enabled": snapshot.commands_enabled,
        "gamemode": gamemode_to_str(snapshot.self_player.gamemode),
    })
    .to_string())
}

// ---------------------------------------------------------------------------
// get_world_view — top-down PNG render for multimodal models
// ---------------------------------------------------------------------------

/// Input for the `get_world_view` MCP tool.
#[derive(Deserialize, Default, rmcp::schemars::JsonSchema)]
pub struct GetWorldViewInput {
    /// Half-extent of the top-down view in blocks (1-32). The rendered image
    /// is `(2*radius+1) x (2*radius+1)` blocks.
    #[schemars(range(min = 1, max = 32))]
    pub radius: u8,
    /// Pixels per block (1/2/4/8). Higher values produce a larger image
    /// for multimodal LLMs that prefer higher-resolution input. `0` and
    /// unsupported values fall back to `1` (legacy 1-pixel-per-block
    /// render). Defaults to `1` so existing clients see no change.
    ///
    /// The image dimensions are `(2*radius+1) * scale` per side — for
    /// example `radius=8, scale=4` produces a `68 * 4 = 272x272` PNG.
    #[serde(default)]
    #[schemars(range(min = 1, max = 8))]
    pub scale: u8,
}

/// Render a top-down PNG of the world around the bot and return it as MCP
/// multi-content (image + text annotation).
///
/// The response is a `Vec<Content>` containing:
///
/// 1. The PNG image as base64 (`Content::Image`, mime `image/png`).
/// 2. A text annotation (`Content::Text`) with JSON metadata: centre
///    coordinates, radius, scale, yaw (if known), and snapshot timestamp.
///    Multimodal LLMs can read this side-by-side with the image to anchor
///    pixel coordinates back to world coordinates.
///
/// ## Caching
///
/// Before re-rendering, the function checks
/// [`SharedState::get_world_view_cache`]. If the cached entry's
/// `snapshot_timestamp`, `radius`, and `scale` all match the current
/// request, the cached PNG + annotation are returned without invoking
/// [`render_topdown_enhanced`](crate::mcp::render::render_topdown_enhanced)
/// again. This makes repeated `get_world_view` calls between snapshot
/// ticks effectively free.
///
/// Validates `radius` is in `1..=32` and the bot is online; on error
/// returns a [`BotError`] so rmcp converts it to a standard MCP error
/// response.
pub fn get_world_view(
    state: &Arc<SharedState>,
    radius: u8,
    scale: u8,
) -> Result<Vec<rmcp::model::Content>, BotError> {
    if !(1..=32).contains(&radius) {
        return Err(BotError::InvalidParams(format!(
            "radius must be 1-32, got {radius}"
        )));
    }
    // scale is clamped inside render_topdown_enhanced; we don't reject
    // out-of-range values here, just fall back to 1 (so existing clients
    // passing `scale=0` or omitting it entirely get the legacy behaviour).
    if !state.is_online() {
        return Err(BotError::Offline("Bot is currently offline".to_string()));
    }

    // Clamp scale to a supported value (1/2/4/8). Invalid inputs fall
    // back to 1 so the annotation JSON accurately reflects what was
    // rendered — clients reading `scale: 0` would otherwise mis-render.
    let scale = if crate::mcp::render::VALID_SCALES.contains(&scale) {
        scale
    } else {
        crate::mcp::render::DEFAULT_SCALE
    };

    let snapshot = state.read_snapshot();
    let snapshot_ts = snapshot.timestamp;

    // Cache hit: same timestamp + radius + scale → return cached bytes.
    if let Some(cache) = state.get_world_view_cache()
        && cache.snapshot_timestamp == snapshot_ts
        && cache.radius == radius
        && cache.scale == scale
    {
        tracing::trace!(
            snapshot_ts,
            radius,
            scale,
            "get_world_view cache hit — returning cached PNG"
        );
        return Ok(vec![
            rmcp::model::Content::image(cache.png_base64, "image/png"),
            rmcp::model::Content::text(cache.annotation_json.clone()),
        ]);
    }

    // Cache miss: re-render.
    tracing::debug!(
        snapshot_ts,
        radius,
        scale,
        "get_world_view cache miss — re-rendering"
    );
    let png_bytes = crate::mcp::render::render_topdown_enhanced(&snapshot, radius, scale)?;
    let encoded = crate::mcp::render::base64_encode(&png_bytes);

    // Build the JSON annotation. Carries enough metadata for a multimodal
    // LLM to anchor pixel coords back to world coords (centre, radius,
    // scale) and to know which way the bot is facing (yaw).
    let (center_x, center_y, center_z) = snapshot
        .self_player
        .position_precise
        .map(|p| (p[0], p[1], p[2]))
        .unwrap_or((
            snapshot.self_player.position.x as f64,
            snapshot.self_player.position.y as f64,
            snapshot.self_player.position.z as f64,
        ));
    let annotation = serde_json::json!({
        "center": [center_x, center_y, center_z],
        "radius": radius,
        "scale": scale,
        "yaw": snapshot.self_player.yaw,
        "snapshot_timestamp": snapshot_ts,
        "image_size": ((2 * radius as u32 + 1) * scale.max(1) as u32),
        "block_count": snapshot.blocks.len(),
        "entity_count": snapshot.entities.len(),
    });
    let annotation_json = annotation.to_string();

    // Store in cache for the next call.
    state.set_world_view_cache(crate::state::WorldViewCache {
        snapshot_timestamp: snapshot_ts,
        radius,
        scale,
        png_base64: encoded.clone(),
        annotation_json: annotation_json.clone(),
    });

    Ok(vec![
        rmcp::model::Content::image(encoded, "image/png"),
        rmcp::model::Content::text(annotation_json),
    ])
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
                position_precise: None,
                yaw: None,
            },
            timestamp: 42,
            chunk_summary: vec![(0, 0), (-1, 0)],
            commands_enabled: None,
            ..Default::default()
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
        let result = get_self_info(&state).unwrap();
        assert!(result.contains("TestBot"));
        assert!(result.contains("player-uuid"));
        assert!(result.contains("18.0")); // health
        assert!(result.contains("15")); // hunger
    }

    #[test]
    fn test_get_self_info_offline() {
        let state = offline_state();
        let result = get_self_info(&state);
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    // -- get_inventory ---------------------------------------------------------

    #[test]
    fn test_get_inventory_online() {
        let state = state_with_snapshot();
        let result = get_inventory(&state).unwrap();
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
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    // -- get_nearby_blocks -----------------------------------------------------

    #[test]
    fn test_get_nearby_blocks_radius_1() {
        let state = state_with_snapshot();
        let result = get_nearby_blocks(&state, 1, None).unwrap();
        // Within radius 1 of (0,64,0): stone at (0,64,0), dirt at (0,65,0)
        assert!(result.contains("stone"));
        assert!(result.contains("dirt"));
        // diamond_ore at (10,64,0) is too far
        assert!(!result.contains("diamond_ore"));
    }

    #[test]
    fn test_get_nearby_blocks_filter() {
        let state = state_with_snapshot();
        let result = get_nearby_blocks(&state, 1, Some("stone".into())).unwrap();
        assert!(result.contains("stone"));
        assert!(!result.contains("dirt"));
    }

    #[test]
    fn test_get_nearby_blocks_empty_filter_acts_as_none() {
        let state = state_with_snapshot();
        let result = get_nearby_blocks(&state, 1, Some("".into())).unwrap();
        assert!(result.contains("stone"));
        assert!(result.contains("dirt"));
    }

    #[test]
    fn test_get_nearby_blocks_offline() {
        let state = offline_state();
        let result = get_nearby_blocks(&state, 5, None);
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    // -- get_nearby_entities ---------------------------------------------------

    #[test]
    fn test_get_nearby_entities_radius_1() {
        let state = state_with_snapshot();
        let result = get_nearby_entities(&state, 1).unwrap();
        assert!(result.contains("zombie"));
        assert!(!result.contains("creeper")); // creeper at (100,64,0) is far
    }

    #[test]
    fn test_get_nearby_entities_large_radius() {
        let state = state_with_snapshot();
        // radius = 100 (the maximum allowed by runtime validation) still
        // catches both nearby and far entities.
        let result = get_nearby_entities(&state, 100).unwrap();
        assert!(result.contains("zombie"));
        assert!(result.contains("creeper"));
    }

    #[test]
    fn test_get_nearby_entities_offline() {
        let state = offline_state();
        let result = get_nearby_entities(&state, 10);
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    // -- get_chunk_summary -----------------------------------------------------

    #[test]
    fn test_get_chunk_summary_online() {
        let state = state_with_snapshot();
        let result = get_chunk_summary(&state).unwrap();
        assert!(result.contains("[0,0]"));
        assert!(result.contains("[-1,0]"));
    }

    #[test]
    fn test_get_chunk_summary_offline() {
        let state = offline_state();
        let result = get_chunk_summary(&state);
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    // -- is_connected ----------------------------------------------------------

    #[test]
    fn test_is_connected_online() {
        let state = state_with_snapshot();
        let result = is_connected(&state).unwrap();
        assert_eq!(result, r#"{"connected":true}"#);
    }

    #[test]
    fn test_is_connected_offline() {
        let state = offline_state();
        let result = is_connected(&state).unwrap();
        assert_eq!(result, r#"{"connected":false}"#);
    }

    // -- get_server_info -------------------------------------------------------

    #[test]
    fn test_get_server_info_online() {
        let state = state_with_snapshot();
        let result = get_server_info(&state).unwrap();
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

        let result = get_server_info(&Arc::new(state)).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).expect("valid JSON");
        assert_eq!(parsed["commands_enabled"], true);
        assert_eq!(parsed["gamemode"], "creative");
    }

    #[test]
    fn test_get_server_info_offline() {
        let state = offline_state();
        let result = get_server_info(&state);
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    // -- get_world_view --------------------------------------------------------

    /// Verify that the multi-content response `[image, text]` is returned
    /// for a valid online call.
    #[test]
    fn test_get_world_view_online_returns_image_and_text() {
        let state = state_with_snapshot();
        let contents = get_world_view(&state, 4, 1).unwrap();
        assert_eq!(
            contents.len(),
            2,
            "response should contain [image, text-annotation]"
        );
        // First content: image.
        match &contents[0].raw {
            rmcp::model::RawContent::Image(img) => {
                assert_eq!(img.mime_type, "image/png");
                assert!(!img.data.is_empty(), "base64 data should be non-empty");
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
        // Second content: text annotation (JSON).
        match &contents[1].raw {
            rmcp::model::RawContent::Text(text) => {
                let parsed: serde_json::Value =
                    serde_json::from_str(&text.text).expect("annotation should be valid JSON");
                assert_eq!(parsed["radius"], 4);
                assert_eq!(parsed["scale"], 1);
                assert!(
                    parsed["center"].is_array(),
                    "annotation should include centre"
                );
                assert!(parsed["snapshot_timestamp"].is_u64());
            }
            other => panic!("expected Text content, got: {other:?}"),
        }
    }

    #[test]
    fn test_get_world_view_offline_returns_error() {
        let state = offline_state();
        let result = get_world_view(&state, 4, 1);
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    #[test]
    fn test_get_world_view_invalid_radius_zero() {
        let state = state_with_snapshot();
        let result = get_world_view(&state, 0, 1);
        assert!(matches!(result, Err(BotError::InvalidParams(_))));
    }

    #[test]
    fn test_get_world_view_invalid_radius_too_large() {
        let state = state_with_snapshot();
        let result = get_world_view(&state, 33, 1);
        assert!(matches!(result, Err(BotError::InvalidParams(_))));
    }

    #[test]
    fn test_get_world_view_radius_boundaries_valid() {
        let state = state_with_snapshot();
        // radius = 1 and radius = 32 should both succeed.
        for radius in [1u8, 32] {
            let contents = get_world_view(&state, radius, 1).unwrap();
            assert!(
                matches!(contents[0].raw, rmcp::model::RawContent::Image(_)),
                "radius {radius} should produce image content"
            );
        }
    }

    /// Different scale values should all succeed and produce valid image
    /// content. Scale 8 produces a much larger PNG than scale 1.
    #[test]
    fn test_get_world_view_scale_values() {
        let state = state_with_snapshot();
        for scale in [1u8, 2, 4, 8] {
            let contents = get_world_view(&state, 4, scale).unwrap();
            match &contents[0].raw {
                rmcp::model::RawContent::Image(img) => {
                    let decoded = base64::engine::general_purpose::STANDARD
                        .decode(&img.data)
                        .expect("base64 decode");
                    assert!(
                        decoded.starts_with(&[0x89, 0x50, 0x4E, 0x47]),
                        "scale {scale} should produce a valid PNG"
                    );
                }
                other => panic!("scale {scale}: expected Image content, got: {other:?}"),
            }
        }
    }

    /// Invalid scale values (0, 3, 5, 9) should fall back to scale=1
    /// rather than erroring — the schema advertises 1-8 but the renderer
    /// clamps gracefully to keep clients from breaking.
    #[test]
    fn test_get_world_view_invalid_scale_falls_back_to_1() {
        let state = state_with_snapshot();
        for invalid_scale in [0u8, 3, 5, 9, 100, 255] {
            let contents = get_world_view(&state, 2, invalid_scale).unwrap();
            // Annotation should report scale=1 (the fallback).
            match &contents[1].raw {
                rmcp::model::RawContent::Text(text) => {
                    let parsed: serde_json::Value =
                        serde_json::from_str(&text.text).expect("valid JSON");
                    assert_eq!(
                        parsed["scale"], 1,
                        "invalid scale {invalid_scale} should fall back to 1"
                    );
                }
                other => panic!("expected Text content, got: {other:?}"),
            }
        }
    }

    /// Two consecutive calls with the same snapshot timestamp + radius +
    /// scale should hit the cache — the second call returns the cached
    /// bytes without re-rendering. We verify this by checking that the
    /// annotation JSON contains the same snapshot_timestamp on both calls.
    #[test]
    fn test_get_world_view_cache_hit_on_repeat_call() {
        let state = state_with_snapshot();
        let first = get_world_view(&state, 4, 1).unwrap();
        let second = get_world_view(&state, 4, 1).unwrap();
        // Both should return 2-element [image, text] responses.
        assert_eq!(first.len(), 2);
        assert_eq!(second.len(), 2);
        // Annotation timestamps should match (same snapshot).
        let first_ts = match &first[1].raw {
            rmcp::model::RawContent::Text(t) => {
                let v: serde_json::Value = serde_json::from_str(&t.text).unwrap();
                v["snapshot_timestamp"].as_u64().unwrap()
            }
            _ => panic!("expected text"),
        };
        let second_ts = match &second[1].raw {
            rmcp::model::RawContent::Text(t) => {
                let v: serde_json::Value = serde_json::from_str(&t.text).unwrap();
                v["snapshot_timestamp"].as_u64().unwrap()
            }
            _ => panic!("expected text"),
        };
        assert_eq!(
            first_ts, second_ts,
            "cache hit should return same timestamp"
        );
    }

    /// A snapshot update (new timestamp) should invalidate the cache —
    /// the second call should re-render with the new timestamp.
    #[test]
    fn test_get_world_view_cache_invalidates_on_snapshot_update() {
        let state = state_with_snapshot();
        let first = get_world_view(&state, 4, 1).unwrap();
        let first_ts = match &first[1].raw {
            rmcp::model::RawContent::Text(t) => {
                let v: serde_json::Value = serde_json::from_str(&t.text).unwrap();
                v["snapshot_timestamp"].as_u64().unwrap()
            }
            _ => panic!("expected text"),
        };

        // Update the snapshot with a new timestamp.
        let mut snap = state.read_snapshot().as_ref().clone();
        snap.timestamp = first_ts + 1;
        state.update_snapshot(snap);

        let second = get_world_view(&state, 4, 1).unwrap();
        let second_ts = match &second[1].raw {
            rmcp::model::RawContent::Text(t) => {
                let v: serde_json::Value = serde_json::from_str(&t.text).unwrap();
                v["snapshot_timestamp"].as_u64().unwrap()
            }
            _ => panic!("expected text"),
        };
        assert_ne!(
            first_ts, second_ts,
            "snapshot update should invalidate cache"
        );
    }

    /// A change in radius or scale should also invalidate the cache.
    #[test]
    fn test_get_world_view_cache_invalidates_on_radius_change() {
        let state = state_with_snapshot();
        let first = get_world_view(&state, 4, 1).unwrap();
        let second = get_world_view(&state, 8, 1).unwrap();
        let first_r = match &first[1].raw {
            rmcp::model::RawContent::Text(t) => {
                let v: serde_json::Value = serde_json::from_str(&t.text).unwrap();
                v["radius"].as_u64().unwrap()
            }
            _ => panic!("expected text"),
        };
        let second_r = match &second[1].raw {
            rmcp::model::RawContent::Text(t) => {
                let v: serde_json::Value = serde_json::from_str(&t.text).unwrap();
                v["radius"].as_u64().unwrap()
            }
            _ => panic!("expected text"),
        };
        assert_ne!(first_r, second_r, "radius change should re-render");
        assert_eq!(first_r, 4);
        assert_eq!(second_r, 8);
    }

    // ── clamp_to_i32 (P0-#1) ───────────────────────────────────────

    /// `clamp_to_i32` is the saturating cast used in `get_nearby_blocks` /
    /// `get_nearby_entities` to keep `u32` radius values above `i32::MAX`
    /// from silently wrapping to negative (which would make the Chebyshev
    /// filter return nothing).
    #[test]
    fn test_radius_clamp_overflow() {
        // 5_000_000_000 exceeds u32::MAX (4_294_967_295) so we use
        // `u32::MAX` directly to exercise the saturate branch.
        assert_eq!(clamp_to_i32(u32::MAX), i32::MAX);
        assert_eq!(clamp_to_i32(4_000_000_000_u32), i32::MAX);
        assert_eq!(clamp_to_i32(100_u32), 100);
        assert_eq!(clamp_to_i32(i32::MAX as u32), i32::MAX);
    }

    /// End-to-end: an oversized `u32` radius is now rejected at runtime
    /// (not silently clamped). The `#[schemars(range(min = 1, max = 100))]`
    /// JSON Schema annotation is now enforced.
    #[test]
    fn test_get_nearby_blocks_oversized_radius_does_not_panic() {
        let state = state_with_snapshot();
        // u32::MAX is rejected by runtime bounds check instead of being
        // silently clamped (and potentially wrapping via `as i32`).
        let result = get_nearby_blocks(&state, u32::MAX, None);
        assert!(
            result.is_err(),
            "u32::MAX must be rejected by radius validation, got: {result:?}"
        );
        // Also verify the error is InvalidParams, not a panic.
        assert!(matches!(result, Err(BotError::InvalidParams(_))));
    }

    /// Same regression for `get_nearby_entities`.
    #[test]
    fn test_get_nearby_entities_oversized_radius_does_not_panic() {
        let state = state_with_snapshot();
        let result = get_nearby_entities(&state, u32::MAX);
        assert!(
            result.is_err(),
            "u32::MAX must be rejected by radius validation, got: {result:?}"
        );
        assert!(matches!(result, Err(BotError::InvalidParams(_))));
    }
}
