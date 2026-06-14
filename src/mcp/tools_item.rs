//! MCP tools for item / inventory management.
//!
//! Each tool validates inputs, checks online status, and dispatches a
//! [`BotCommand`](crate::types::BotCommand) through the bot command channel.
//!
//! # Parameter structs
//!
//! We implement [`rmcp::schemars::JsonSchema`] manually using schemars v1.2.1
//! API (bundled by rmcp 1.7.0) to avoid version conflicts with the project's
//! schemars v0.8 dependency.

use std::borrow::Cow;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::channel::BotCommandSender;
use crate::state::SharedState;
use crate::types::{BotCommand, ToolType};

// ── Helper ──────────────────────────────────────────────────────────────────

fn schema_from_json(v: Value) -> rmcp::schemars::Schema {
    let map: Map<String, Value> = v.as_object().cloned().unwrap_or_default();
    rmcp::schemars::Schema::from(map)
}

// ── switch_hotbar_slot ─────────────────────────────────────────────────────

/// Input for the `switch_hotbar_slot` MCP tool.
#[derive(Deserialize, Default)]
pub struct SwitchHotbarSlotInput {
    pub slot: u8,
}

impl rmcp::schemars::JsonSchema for SwitchHotbarSlotInput {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("SwitchHotbarSlotInput")
    }

    fn json_schema(
        _gen: &mut rmcp::schemars::SchemaGenerator,
    ) -> rmcp::schemars::Schema {
        schema_from_json(json!({
            "type": "object",
            "properties": {
                "slot": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 8,
                    "description": "Hotbar slot to activate (0-8)"
                }
            },
            "required": ["slot"],
            "additionalProperties": false
        }))
    }
}

/// Handle `switch_hotbar_slot` MCP tool.
pub async fn handle_switch_hotbar_slot(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    input: SwitchHotbarSlotInput,
) -> String {
    if input.slot > 8 {
        return format!(
            r#"{{"success":false,"error":"Hotbar slot must be 0-8, got {}"}}"#,
            input.slot
        );
    }
    if !state.is_online() {
        return r#"{"success":false,"error":"Bot is not connected to a server"}"#
            .to_string();
    }

    let cmd = BotCommand::SwitchHotbarSlot(input.slot);
    match sender.send_command(cmd).await {
        Ok(result) => serde_json::to_string(&result).unwrap_or_else(|e| {
            format!(r#"{{"success":false,"error":"Serialization error: {e}"}}"#)
        }),
        Err(e) => format!(r#"{{"success":false,"error":"{e}"}}"#),
    }
}

// ── drop_item ──────────────────────────────────────────────────────────────

/// Input for the `drop_item` MCP tool.
#[derive(Deserialize, Default)]
pub struct DropItemInput {
    pub slot: u8,
    pub count: Option<u8>,
}

impl rmcp::schemars::JsonSchema for DropItemInput {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("DropItemInput")
    }

    fn json_schema(
        _gen: &mut rmcp::schemars::SchemaGenerator,
    ) -> rmcp::schemars::Schema {
        schema_from_json(json!({
            "type": "object",
            "properties": {
                "slot": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 35,
                    "description": "Inventory slot to drop from (0-35)"
                },
                "count": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 64,
                    "description": "Number of items to drop (default 1)"
                }
            },
            "required": ["slot"],
            "additionalProperties": false
        }))
    }
}

/// Handle `drop_item` MCP tool.
pub async fn handle_drop_item(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    input: DropItemInput,
) -> String {
    if input.slot > 35 {
        return format!(
            r#"{{"success":false,"error":"Inventory slot must be 0-35, got {}"}}"#,
            input.slot
        );
    }
    let count = input.count.unwrap_or(1);
    if count == 0 {
        return r#"{"success":false,"error":"Count must be greater than 0"}"#
            .to_string();
    }
    if !state.is_online() {
        return r#"{"success":false,"error":"Bot is not connected to a server"}"#
            .to_string();
    }

    let cmd = BotCommand::DropItem(input.slot, count);
    match sender.send_command(cmd).await {
        Ok(result) => serde_json::to_string(&result).unwrap_or_else(|e| {
            format!(r#"{{"success":false,"error":"Serialization error: {e}"}}"#)
        }),
        Err(e) => format!(r#"{{"success":false,"error":"{e}"}}"#),
    }
}

// ── use_item ───────────────────────────────────────────────────────────────

/// Input for the `use_item` MCP tool.
#[derive(Deserialize, Default)]
pub struct UseItemInput {
    pub item_slot: Option<u8>,
}

impl rmcp::schemars::JsonSchema for UseItemInput {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("UseItemInput")
    }

    fn json_schema(
        _gen: &mut rmcp::schemars::SchemaGenerator,
    ) -> rmcp::schemars::Schema {
        schema_from_json(json!({
            "type": "object",
            "properties": {
                "item_slot": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 8,
                    "description": "Optional hotbar slot (0-8). Uses currently held item if omitted."
                }
            },
            "additionalProperties": false
        }))
    }
}

/// Handle `use_item` MCP tool.
pub async fn handle_use_item(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    input: UseItemInput,
) -> String {
    if let Some(slot) = input.item_slot {
        if slot > 8 {
            return format!(
                r#"{{"success":false,"error":"item_slot must be 0-8, got {slot}"}}"#
            );
        }
    }
    if !state.is_online() {
        return r#"{"success":false,"error":"Bot is not connected to a server"}"#
            .to_string();
    }

    let cmd = BotCommand::UseItem;
    match sender.send_command(cmd).await {
        Ok(result) => serde_json::to_string(&result).unwrap_or_else(|e| {
            format!(r#"{{"success":false,"error":"Serialization error: {e}"}}"#)
        }),
        Err(e) => format!(r#"{{"success":false,"error":"{e}"}}"#),
    }
}

// ── equip_tool ─────────────────────────────────────────────────────────────

/// Input for the `equip_tool` MCP tool.
#[derive(Deserialize, Default)]
pub struct EquipToolInput {
    pub tool_type: String,
    pub material_preference: Option<String>,
}

impl rmcp::schemars::JsonSchema for EquipToolInput {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("EquipToolInput")
    }

    fn json_schema(
        _gen: &mut rmcp::schemars::SchemaGenerator,
    ) -> rmcp::schemars::Schema {
        schema_from_json(json!({
            "type": "object",
            "properties": {
                "tool_type": {
                    "type": "string",
                    "description": "Tool type. One of: pickaxe, axe, shovel, sword, shears, hand",
                    "enum": ["pickaxe", "axe", "shovel", "sword", "shears", "hand"]
                },
                "material_preference": {
                    "type": "string",
                    "description": "Optional material tier (e.g. diamond, netherite, iron, stone, wood, gold)"
                }
            },
            "required": ["tool_type"],
            "additionalProperties": false
        }))
    }
}

/// Parse a tool type string into a [`ToolType`] (case-insensitive).
pub fn parse_tool_type(s: &str) -> Option<ToolType> {
    match s.to_lowercase().as_str() {
        "pickaxe" => Some(ToolType::Pickaxe),
        "axe" => Some(ToolType::Axe),
        "shovel" => Some(ToolType::Shovel),
        "sword" => Some(ToolType::Sword),
        "shears" => Some(ToolType::Shears),
        "hand" => Some(ToolType::Hand),
        _ => None,
    }
}

/// Handle `equip_tool` MCP tool.
pub async fn handle_equip_tool(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    input: EquipToolInput,
) -> String {
    let tool = match parse_tool_type(&input.tool_type) {
        Some(t) => t,
        None => {
            return format!(
                r#"{{"success":false,"error":"Unknown tool type: '{}'. Valid types: pickaxe, axe, shovel, sword, shears, hand"}}"#,
                input.tool_type
            );
        }
    };

    if !state.is_online() {
        return r#"{"success":false,"error":"Bot is not connected to a server"}"#
            .to_string();
    }

    let cmd = BotCommand::EquipTool(tool);
    match sender.send_command(cmd).await {
        Ok(result) => {
            let mut json =
                serde_json::to_value(&result).unwrap_or_default();
            if let Some(mat) = input.material_preference {
                if let Some(obj) = json.as_object_mut() {
                    obj.insert(
                        "material_preference".to_string(),
                        Value::String(mat),
                    );
                }
            }
            serde_json::to_string(&json).unwrap_or_else(|e| {
                format!(r#"{{"success":false,"error":"Serialization error: {e}"}}"#)
            })
        }
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
                let _ = wrapped
                    .respond_to
                    .send(Ok(crate::types::BotResult {
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

    #[tokio::test]
    async fn test_switch_hotbar_slot_offline() {
        let (state, sender) = setup();
        let input = SwitchHotbarSlotInput { slot: 0 };
        let result =
            handle_switch_hotbar_slot(&state, &sender, input).await;
        assert!(result.contains("not connected"));
    }

    #[tokio::test]
    async fn test_switch_hotbar_slot_invalid() {
        let (state, sender) = setup();
        make_online(&state);
        let input = SwitchHotbarSlotInput { slot: 9 };
        let result =
            handle_switch_hotbar_slot(&state, &sender, input).await;
        assert!(result.contains("must be 0-8"));
    }

    #[tokio::test]
    async fn test_switch_hotbar_slot_valid_range() {
        let (state, sender) = setup();
        make_online(&state);
        for slot in 0..=8u8 {
            let result = handle_switch_hotbar_slot(
                &state,
                &sender,
                SwitchHotbarSlotInput { slot },
            )
            .await;
            let _: Value = serde_json::from_str(&result).expect("valid JSON");
        }
    }

    #[tokio::test]
    async fn test_drop_item_offline() {
        let (state, sender) = setup();
        let input = DropItemInput {
            slot: 0,
            count: Some(1),
        };
        let result = handle_drop_item(&state, &sender, input).await;
        assert!(result.contains("not connected"));
    }

    #[tokio::test]
    async fn test_drop_item_invalid_slot() {
        let (state, sender) = setup();
        make_online(&state);
        let input = DropItemInput {
            slot: 36,
            count: Some(1),
        };
        let result = handle_drop_item(&state, &sender, input).await;
        assert!(result.contains("must be 0-35"));
    }

    #[tokio::test]
    async fn test_drop_item_zero_count() {
        let (state, sender) = setup();
        make_online(&state);
        let input = DropItemInput {
            slot: 0,
            count: Some(0),
        };
        let result = handle_drop_item(&state, &sender, input).await;
        assert!(result.contains("greater than 0"));
    }

    #[tokio::test]
    async fn test_drop_item_default_count() {
        let (state, sender) = setup();
        make_online(&state);
        let input = DropItemInput {
            slot: 5,
            count: None,
        };
        let result = handle_drop_item(&state, &sender, input).await;
        let _: Value = serde_json::from_str(&result).expect("valid JSON");
    }

    #[tokio::test]
    async fn test_use_item_offline() {
        let (state, sender) = setup();
        let input = UseItemInput { item_slot: None };
        let result = handle_use_item(&state, &sender, input).await;
        assert!(result.contains("not connected"));
    }

    #[tokio::test]
    async fn test_use_item_no_slot() {
        let (state, sender) = setup();
        make_online(&state);
        let input = UseItemInput { item_slot: None };
        let result = handle_use_item(&state, &sender, input).await;
        let _: Value = serde_json::from_str(&result).expect("valid JSON");
    }

    #[tokio::test]
    async fn test_use_item_invalid_slot() {
        let (state, sender) = setup();
        make_online(&state);
        let input = UseItemInput {
            item_slot: Some(10),
        };
        let result = handle_use_item(&state, &sender, input).await;
        assert!(result.contains("must be 0-8"));
    }

    #[tokio::test]
    async fn test_equip_tool_offline() {
        let (state, sender) = setup();
        let input = EquipToolInput {
            tool_type: "pickaxe".into(),
            material_preference: None,
        };
        let result =
            handle_equip_tool(&state, &sender, input).await;
        assert!(result.contains("not connected"));
    }

    #[tokio::test]
    async fn test_equip_tool_unknown_type() {
        let (state, sender) = setup();
        make_online(&state);
        let input = EquipToolInput {
            tool_type: "hoe".into(),
            material_preference: None,
        };
        let result =
            handle_equip_tool(&state, &sender, input).await;
        assert!(result.contains("Unknown tool type"));
    }

    #[tokio::test]
    async fn test_equip_tool_valid_types() {
        let (state, sender) = setup();
        make_online(&state);
        for tt in ["pickaxe", "axe", "shovel", "sword", "shears", "hand"] {
            let input = EquipToolInput {
                tool_type: tt.into(),
                material_preference: None,
            };
            let result =
                handle_equip_tool(&state, &sender, input).await;
            let _: Value = serde_json::from_str(&result).expect("valid JSON");
        }
    }

    #[tokio::test]
    async fn test_equip_tool_with_material_preference() {
        let (state, sender) = make_echo_channel();
        make_online(&state);
        let input = EquipToolInput {
            tool_type: "pickaxe".into(),
            material_preference: Some("diamond".into()),
        };
        let result =
            handle_equip_tool(&state, &sender, input).await;
        assert!(result.contains("material_preference"));
    }

    #[test]
    fn test_parse_tool_type_all_variants() {
        assert_eq!(parse_tool_type("pickaxe"), Some(ToolType::Pickaxe));
        assert_eq!(parse_tool_type("axe"), Some(ToolType::Axe));
        assert_eq!(parse_tool_type("shovel"), Some(ToolType::Shovel));
        assert_eq!(parse_tool_type("sword"), Some(ToolType::Sword));
        assert_eq!(parse_tool_type("shears"), Some(ToolType::Shears));
        assert_eq!(parse_tool_type("hand"), Some(ToolType::Hand));
    }

    #[test]
    fn test_parse_tool_type_case_insensitive() {
        assert_eq!(parse_tool_type("PICKAXE"), Some(ToolType::Pickaxe));
        assert_eq!(parse_tool_type("SWORD"), Some(ToolType::Sword));
    }

    #[test]
    fn test_parse_tool_type_unknown() {
        assert_eq!(parse_tool_type("hoe"), None);
        assert_eq!(parse_tool_type(""), None);
    }
}
