//! MCP tools for block operations (break, place, use item on block).
//!
//! Each tool validates coordinates through [`validate_block_pos`], checks
//! online status, and dispatches a [`BotCommand`](crate::types::BotCommand)
//! through the bot command channel.
//!
//! # Parameter structs
//!
//! We implement [`rmcp::schemars::JsonSchema`] manually using schemars v1.2.1
//! API (bundled by rmcp 1.7.0) to avoid version conflicts with the project's
//! schemars v0.8 dependency.

use std::borrow::Cow;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::channel::BotCommandSender;
use crate::command_validate::validate_block_pos;
use crate::state::SharedState;
use crate::types::{BlockPos, BotCommand};

// ── Helper ──────────────────────────────────────────────────────────────────

fn schema_from_json(v: Value) -> rmcp::schemars::Schema {
    let map: Map<String, Value> = v.as_object().cloned().unwrap_or_default();
    rmcp::schemars::Schema::from(map)
}

// ── break_block ────────────────────────────────────────────────────────────

/// Input for the `break_block` MCP tool.
#[derive(Deserialize, Default)]
pub struct BreakBlockInput {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub use_best_tool: Option<bool>,
}

impl rmcp::schemars::JsonSchema for BreakBlockInput {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("BreakBlockInput")
    }

    fn json_schema(_gen: &mut rmcp::schemars::SchemaGenerator) -> rmcp::schemars::Schema {
        schema_from_json(json!({
            "type": "object",
            "properties": {
                "x": {
                    "type": "integer",
                    "description": "X coordinate of the block to break"
                },
                "y": {
                    "type": "integer",
                    "description": "Y coordinate of the block to break"
                },
                "z": {
                    "type": "integer",
                    "description": "Z coordinate of the block to break"
                },
                "use_best_tool": {
                    "type": "boolean",
                    "description": "If true, automatically equip the best tool for the block before mining (compound mine_block flow)"
                }
            },
            "required": ["x", "y", "z"],
            "additionalProperties": false
        }))
    }
}

/// Handle `break_block` MCP tool.
///
/// Validates coordinates, checks online status, then sends
/// [`BotCommand::BreakBlock`]. When `use_best_tool` is `true`, the
/// response includes a flag so the bot executor can trigger the compound
/// mine_block flow (tool selection → movement → break).
pub async fn handle_break_block(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    input: BreakBlockInput,
) -> String {
    // Validate coordinates are within world bounds
    if let Err(e) = validate_block_pos(&BlockPos::new(input.x, input.y, input.z)) {
        return format!(r#"{{"success":false,"error":"{e}"}}"#);
    }

    if !state.is_online() {
        return r#"{"success":false,"error":"Bot is not connected to a server"}"#.to_string();
    }

    let cmd = BotCommand::BreakBlock(BlockPos::new(input.x, input.y, input.z));
    match sender.send_command(cmd).await {
        Ok(result) => {
            let mut json = serde_json::to_value(&result).unwrap_or_default();
            if let Some(obj) = json.as_object_mut() {
                obj.insert(
                    "use_best_tool".to_string(),
                    Value::Bool(input.use_best_tool.unwrap_or(false)),
                );
            }
            serde_json::to_string(&json).unwrap_or_else(|e| {
                format!(r#"{{"success":false,"error":"Serialization error: {e}"}}"#)
            })
        }
        Err(e) => format!(r#"{{"success":false,"error":"{e}"}}"#),
    }
}

// ── place_block ────────────────────────────────────────────────────────────

/// Input for the `place_block` MCP tool.
#[derive(Deserialize, Default)]
pub struct PlaceBlockInput {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub item_slot: u8,
}

impl rmcp::schemars::JsonSchema for PlaceBlockInput {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("PlaceBlockInput")
    }

    fn json_schema(_gen: &mut rmcp::schemars::SchemaGenerator) -> rmcp::schemars::Schema {
        schema_from_json(json!({
            "type": "object",
            "properties": {
                "x": {
                    "type": "integer",
                    "description": "X coordinate to place the block at"
                },
                "y": {
                    "type": "integer",
                    "description": "Y coordinate to place the block at"
                },
                "z": {
                    "type": "integer",
                    "description": "Z coordinate to place the block at"
                },
                "item_slot": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 8,
                    "description": "Hotbar slot (0-8) containing the block to place"
                }
            },
            "required": ["x", "y", "z", "item_slot"],
            "additionalProperties": false
        }))
    }
}

/// Handle `place_block` MCP tool.
///
/// Validates coordinates and slot, checks online status, then sends
/// [`BotCommand::PlaceBlock`] with the target position. The `item_slot`
/// is encoded as `"slot:N"` in the block type field so the bot executor
/// can resolve the actual block type from the player's inventory.
pub async fn handle_place_block(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    input: PlaceBlockInput,
) -> String {
    // Validate coordinates are within world bounds
    if let Err(e) = validate_block_pos(&BlockPos::new(input.x, input.y, input.z)) {
        return format!(r#"{{"success":false,"error":"{e}"}}"#);
    }

    // Validate item slot is in hotbar range
    if input.item_slot > 8 {
        return format!(
            r#"{{"success":false,"error":"item_slot must be 0-8, got {}"}}"#,
            input.item_slot
        );
    }

    if !state.is_online() {
        return r#"{"success":false,"error":"Bot is not connected to a server"}"#.to_string();
    }

    let cmd = BotCommand::PlaceBlock(
        BlockPos::new(input.x, input.y, input.z),
        format!("slot:{}", input.item_slot),
    );
    match sender.send_command(cmd).await {
        Ok(result) => serde_json::to_string(&result).unwrap_or_else(|e| {
            format!(r#"{{"success":false,"error":"Serialization error: {e}"}}"#)
        }),
        Err(e) => format!(r#"{{"success":false,"error":"{e}"}}"#),
    }
}

// ── use_item_on_block ──────────────────────────────────────────────────────

/// Input for the `use_item_on_block` MCP tool.
#[derive(Deserialize, Default)]
pub struct UseItemOnBlockInput {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub item_slot: Option<u8>,
}

impl rmcp::schemars::JsonSchema for UseItemOnBlockInput {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("UseItemOnBlockInput")
    }

    fn json_schema(_gen: &mut rmcp::schemars::SchemaGenerator) -> rmcp::schemars::Schema {
        schema_from_json(json!({
            "type": "object",
            "properties": {
                "x": {
                    "type": "integer",
                    "description": "X coordinate of the block to interact with"
                },
                "y": {
                    "type": "integer",
                    "description": "Y coordinate of the block to interact with"
                },
                "z": {
                    "type": "integer",
                    "description": "Z coordinate of the block to interact with"
                },
                "item_slot": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 8,
                    "description": "Optional hotbar slot (0-8) to select before using. Uses currently held item if omitted."
                }
            },
            "required": ["x", "y", "z"],
            "additionalProperties": false
        }))
    }
}

/// Handle `use_item_on_block` MCP tool.
///
/// Validates coordinates and optional slot, checks online status, then sends
/// [`BotCommand::UseItemOnBlock`]. If `item_slot` is provided, the bot
/// executor should switch to that slot before interacting.
pub async fn handle_use_item_on_block(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    input: UseItemOnBlockInput,
) -> String {
    // Validate coordinates are within world bounds
    if let Err(e) = validate_block_pos(&BlockPos::new(input.x, input.y, input.z)) {
        return format!(r#"{{"success":false,"error":"{e}"}}"#);
    }

    // Validate optional item slot
    if let Some(slot) = input.item_slot
        && slot > 8
    {
        return format!(r#"{{"success":false,"error":"item_slot must be 0-8, got {slot}"}}"#);
    }

    if !state.is_online() {
        return r#"{"success":false,"error":"Bot is not connected to a server"}"#.to_string();
    }

    let cmd = BotCommand::UseItemOnBlock(BlockPos::new(input.x, input.y, input.z));
    match sender.send_command(cmd).await {
        Ok(result) => serde_json::to_string(&result).unwrap_or_else(|e| {
            format!(r#"{{"success":false,"error":"Serialization error: {e}"}}"#)
        }),
        Err(e) => format!(r#"{{"success":false,"error":"{e}"}}"#),
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::create_command_channel;
    use crate::config::AppConfig;

    fn setup() -> (Arc<SharedState>, BotCommandSender) {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, _receiver) = create_command_channel(4);
        (state, sender)
    }

    /// Create a channel where the receiver echoes back a successful BotResult.
    fn make_echo_channel() -> (Arc<SharedState>, BotCommandSender) {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, mut receiver) = create_command_channel(4);

        tokio::spawn(async move {
            while let Some(wrapped) = receiver.recv().await {
                let msg = format!("executed: {:?}", wrapped.command);
                let _ = wrapped.respond_to.send(Ok(crate::types::BotResult {
                    success: true,
                    message: msg,
                    data: None,
                }));
            }
        });

        (state, sender)
    }

    fn make_online(state: &SharedState) {
        state.set_online(true);
    }

    // ── break_block ────────────────────────────────────────────────

    #[tokio::test]
    async fn test_break_block_offline() {
        let (state, sender) = setup();
        let input = BreakBlockInput {
            x: 0,
            y: 64,
            z: 0,
            use_best_tool: None,
        };
        let result = handle_break_block(&state, &sender, input).await;
        assert!(result.contains("not connected"));
    }

    #[tokio::test]
    async fn test_break_block_invalid_coords() {
        let (state, sender) = setup();
        make_online(&state);
        let input = BreakBlockInput {
            x: 0,
            y: 500,
            z: 0,
            use_best_tool: None,
        };
        let result = handle_break_block(&state, &sender, input).await;
        assert!(result.contains("out of bounds") || result.contains("out of range"));
    }

    #[tokio::test]
    async fn test_break_block_valid() {
        let (state, sender) = setup();
        make_online(&state);
        let input = BreakBlockInput {
            x: 10,
            y: 64,
            z: -5,
            use_best_tool: None,
        };
        let result = handle_break_block(&state, &sender, input).await;
        let _: Value = serde_json::from_str(&result).expect("valid JSON");
    }

    #[tokio::test]
    async fn test_break_block_with_best_tool() {
        let (state, sender) = make_echo_channel();
        make_online(&state);
        let input = BreakBlockInput {
            x: 10,
            y: 64,
            z: -5,
            use_best_tool: Some(true),
        };
        let result = handle_break_block(&state, &sender, input).await;
        let json: Value = serde_json::from_str(&result).expect("valid JSON");
        assert_eq!(json.get("use_best_tool"), Some(&Value::Bool(true)));
    }

    #[tokio::test]
    async fn test_break_block_with_best_tool_false() {
        let (state, sender) = make_echo_channel();
        make_online(&state);
        let input = BreakBlockInput {
            x: 10,
            y: 64,
            z: -5,
            use_best_tool: Some(false),
        };
        let result = handle_break_block(&state, &sender, input).await;
        let json: Value = serde_json::from_str(&result).expect("valid JSON");
        assert_eq!(json.get("use_best_tool"), Some(&Value::Bool(false)));
    }

    // ── place_block ────────────────────────────────────────────────

    #[tokio::test]
    async fn test_place_block_offline() {
        let (state, sender) = setup();
        let input = PlaceBlockInput {
            x: 0,
            y: 64,
            z: 0,
            item_slot: 0,
        };
        let result = handle_place_block(&state, &sender, input).await;
        assert!(result.contains("not connected"));
    }

    #[tokio::test]
    async fn test_place_block_invalid_coords() {
        let (state, sender) = setup();
        make_online(&state);
        let input = PlaceBlockInput {
            x: 0,
            y: -100,
            z: 0,
            item_slot: 0,
        };
        let result = handle_place_block(&state, &sender, input).await;
        assert!(result.contains("out of bounds") || result.contains("out of range"));
    }

    #[tokio::test]
    async fn test_place_block_invalid_slot() {
        let (state, sender) = setup();
        make_online(&state);
        let input = PlaceBlockInput {
            x: 0,
            y: 64,
            z: 0,
            item_slot: 9,
        };
        let result = handle_place_block(&state, &sender, input).await;
        assert!(result.contains("must be 0-8"));
    }

    #[tokio::test]
    async fn test_place_block_valid() {
        let (state, sender) = setup();
        make_online(&state);
        let input = PlaceBlockInput {
            x: 5,
            y: 65,
            z: 10,
            item_slot: 3,
        };
        let result = handle_place_block(&state, &sender, input).await;
        let _: Value = serde_json::from_str(&result).expect("valid JSON");
    }

    #[tokio::test]
    async fn test_place_block_min_slot_valid() {
        let (state, sender) = setup();
        make_online(&state);
        let input = PlaceBlockInput {
            x: 0,
            y: 64,
            z: 0,
            item_slot: 0,
        };
        let result = handle_place_block(&state, &sender, input).await;
        let _: Value = serde_json::from_str(&result).expect("valid JSON");
    }

    #[tokio::test]
    async fn test_place_block_max_slot_valid() {
        let (state, sender) = setup();
        make_online(&state);
        let input = PlaceBlockInput {
            x: 0,
            y: 64,
            z: 0,
            item_slot: 8,
        };
        let result = handle_place_block(&state, &sender, input).await;
        let _: Value = serde_json::from_str(&result).expect("valid JSON");
    }

    // ── use_item_on_block ──────────────────────────────────────────

    #[tokio::test]
    async fn test_use_item_on_block_offline() {
        let (state, sender) = setup();
        let input = UseItemOnBlockInput {
            x: 0,
            y: 64,
            z: 0,
            item_slot: None,
        };
        let result = handle_use_item_on_block(&state, &sender, input).await;
        assert!(result.contains("not connected"));
    }

    #[tokio::test]
    async fn test_use_item_on_block_invalid_coords() {
        let (state, sender) = setup();
        make_online(&state);
        let input = UseItemOnBlockInput {
            x: 99_999_999,
            y: 0,
            z: 0,
            item_slot: None,
        };
        let result = handle_use_item_on_block(&state, &sender, input).await;
        assert!(result.contains("out of bounds") || result.contains("out of range"));
    }

    #[tokio::test]
    async fn test_use_item_on_block_invalid_slot() {
        let (state, sender) = setup();
        make_online(&state);
        let input = UseItemOnBlockInput {
            x: 0,
            y: 64,
            z: 0,
            item_slot: Some(10),
        };
        let result = handle_use_item_on_block(&state, &sender, input).await;
        assert!(result.contains("must be 0-8"));
    }

    #[tokio::test]
    async fn test_use_item_on_block_valid_no_slot() {
        let (state, sender) = setup();
        make_online(&state);
        let input = UseItemOnBlockInput {
            x: 0,
            y: 64,
            z: 0,
            item_slot: None,
        };
        let result = handle_use_item_on_block(&state, &sender, input).await;
        let _: Value = serde_json::from_str(&result).expect("valid JSON");
    }

    #[tokio::test]
    async fn test_use_item_on_block_valid_with_slot() {
        let (state, sender) = setup();
        make_online(&state);
        let input = UseItemOnBlockInput {
            x: 1,
            y: 64,
            z: 1,
            item_slot: Some(4),
        };
        let result = handle_use_item_on_block(&state, &sender, input).await;
        let _: Value = serde_json::from_str(&result).expect("valid JSON");
    }
}
