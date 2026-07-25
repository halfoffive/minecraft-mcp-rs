//! MCP tools for item / inventory management.
//!
//! Each tool validates inputs, checks online status, and dispatches a
//! [`BotCommand`] through the bot command channel.

use std::sync::Arc;

use serde::Deserialize;

use crate::channel::BotCommandSender;
use crate::error::BotError;
use crate::state::SharedState;
use crate::types::{BotCommand, MaterialTier, ToolType};

// ── switch_hotbar_slot ─────────────────────────────────────────────────────

/// Input for the `switch_hotbar_slot` MCP tool.
#[derive(Deserialize, Default, rmcp::schemars::JsonSchema)]
pub struct SwitchHotbarSlotInput {
    /// Hotbar slot to activate (0-8).
    #[schemars(range(min = 0, max = 8))]
    pub slot: u8,
}

/// Handle `switch_hotbar_slot` MCP tool.
pub async fn handle_switch_hotbar_slot(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    input: SwitchHotbarSlotInput,
) -> Result<String, BotError> {
    if input.slot > 8 {
        return Err(BotError::InvalidParams(format!(
            "Hotbar slot must be 0-8, got {}",
            input.slot
        )));
    }
    if !state.is_online() {
        return Err(BotError::Offline(
            "Bot is not connected to a server".to_string(),
        ));
    }

    let cmd = BotCommand::SwitchHotbarSlot(input.slot);
    match sender.send_command(cmd).await {
        Ok(result) => serde_json::to_string(&result)
            .map_err(|e| BotError::Internal(format!("Serialization error: {e}"))),
        Err(e) => Err(e),
    }
}

// ── drop_item ──────────────────────────────────────────────────────────────

/// Input for the `drop_item` MCP tool.
#[derive(Deserialize, Default, rmcp::schemars::JsonSchema)]
pub struct DropItemInput {
    /// Inventory slot to drop from (0-35).
    #[schemars(range(min = 0, max = 35))]
    pub slot: u8,
    /// Number of items to drop (default 1).
    #[schemars(range(min = 1, max = 64))]
    pub count: Option<u8>,
}

/// Handle `drop_item` MCP tool.
pub async fn handle_drop_item(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    input: DropItemInput,
) -> Result<String, BotError> {
    if input.slot > 35 {
        return Err(BotError::InvalidParams(format!(
            "Inventory slot must be 0-35, got {}",
            input.slot
        )));
    }
    let count = input.count.unwrap_or(1);
    if count < 1 || count > 64 {
        return Err(BotError::InvalidParams(format!(
            "Count must be between 1 and 64, got {count}"
        )));
    }
    if !state.is_online() {
        return Err(BotError::Offline(
            "Bot is not connected to a server".to_string(),
        ));
    }

    let cmd = BotCommand::DropItem(input.slot, count);
    match sender.send_command(cmd).await {
        Ok(result) => serde_json::to_string(&result)
            .map_err(|e| BotError::Internal(format!("Serialization error: {e}"))),
        Err(e) => Err(e),
    }
}

// ── use_item ───────────────────────────────────────────────────────────────

/// Input for the `use_item` MCP tool.
#[derive(Deserialize, Default, rmcp::schemars::JsonSchema)]
pub struct UseItemInput {
    /// Optional hotbar slot (0-8). Uses currently held item if omitted.
    #[schemars(range(min = 0, max = 8))]
    pub item_slot: Option<u8>,
}

/// Handle `use_item` MCP tool.
///
/// If `item_slot` is provided, switches to that hotbar slot before using
/// the item (otherwise the currently held item is used).
pub async fn handle_use_item(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    input: UseItemInput,
) -> Result<String, BotError> {
    if let Some(slot) = input.item_slot
        && slot > 8
    {
        return Err(BotError::InvalidParams(format!(
            "item_slot must be 0-8, got {slot}"
        )));
    }
    if !state.is_online() {
        return Err(BotError::Offline(
            "Bot is not connected to a server".to_string(),
        ));
    }

    // When a slot is requested, send a single atomic UseItemWithSlot command
    // so the switch + use cannot be interleaved with other commands under
    // HTTP transport concurrency. Without a slot, fall back to UseItem.
    let cmd = match input.item_slot {
        Some(slot) => BotCommand::UseItemWithSlot(slot),
        None => BotCommand::UseItem,
    };
    match sender.send_command(cmd).await {
        Ok(result) => serde_json::to_string(&result)
            .map_err(|e| BotError::Internal(format!("Serialization error: {e}"))),
        Err(e) => Err(e),
    }
}

// ── equip_tool ─────────────────────────────────────────────────────────────

/// Input for the `equip_tool` MCP tool.
#[derive(Deserialize, Default, rmcp::schemars::JsonSchema)]
pub struct EquipToolInput {
    /// Tool type. One of: pickaxe, axe, shovel, hoe, sword, shears, hand.
    pub tool_type: String,
    /// Optional material tier (e.g. diamond, netherite, iron, stone, wood, gold).
    pub material_preference: Option<String>,
}

/// Parse a tool type string into a [`ToolType`] (case-insensitive).
pub fn parse_tool_type(s: &str) -> Option<ToolType> {
    match s.to_lowercase().as_str() {
        "pickaxe" => Some(ToolType::Pickaxe),
        "axe" => Some(ToolType::Axe),
        "shovel" => Some(ToolType::Shovel),
        "hoe" => Some(ToolType::Hoe),
        "sword" => Some(ToolType::Sword),
        "shears" => Some(ToolType::Shears),
        "hand" => Some(ToolType::Hand),
        _ => None,
    }
}

/// Parse a material tier string into a [`MaterialTier`] (case-insensitive).
///
/// Accepts the canonical tier names plus the common `wooden` / `golden`
/// aliases. Returns `None` for anything unrecognised so the caller can report
/// an `InvalidParams` error instead of silently ignoring the preference.
pub fn parse_material_tier(s: &str) -> Option<MaterialTier> {
    match s.to_lowercase().as_str() {
        "wood" | "wooden" => Some(MaterialTier::Wood),
        "gold" | "golden" => Some(MaterialTier::Gold),
        "stone" => Some(MaterialTier::Stone),
        "iron" => Some(MaterialTier::Iron),
        "diamond" => Some(MaterialTier::Diamond),
        "netherite" => Some(MaterialTier::Netherite),
        _ => None,
    }
}

/// Handle `equip_tool` MCP tool.
pub async fn handle_equip_tool(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    input: EquipToolInput,
) -> Result<String, BotError> {
    let tool = match parse_tool_type(&input.tool_type) {
        Some(t) => t,
        None => {
            return Err(BotError::InvalidParams(format!(
                "Unknown tool type: '{}'. Valid types: pickaxe, axe, shovel, hoe, sword, shears, hand",
                input.tool_type
            )));
        }
    };

    // Parse the optional material preference into a minimum tier. An invalid
    // value is a client error rather than being silently ignored.
    let material = match input.material_preference.as_deref() {
        Some(s) => match parse_material_tier(s) {
            Some(m) => Some(m),
            None => {
                return Err(BotError::InvalidParams(format!(
                    "Unknown material preference: '{s}'. Valid tiers: wood, gold, stone, iron, diamond, netherite"
                )));
            }
        },
        None => None,
    };

    if !state.is_online() {
        return Err(BotError::Offline(
            "Bot is not connected to a server".to_string(),
        ));
    }

    // Route to the material-aware command only when a preference is given, so
    // callers without one keep the plain EquipTool behaviour.
    let cmd = match material {
        Some(m) => BotCommand::EquipToolWithMaterial(tool, m),
        None => BotCommand::EquipTool(tool),
    };
    let result = sender.send_command(cmd).await?;
    serde_json::to_string(&result)
        .map_err(|e| BotError::Internal(format!("Serialization error: {e}")))
}

// ── collect_items ──────────────────────────────────────────────────────────

/// Input for the `collect_items` MCP tool.
#[derive(Deserialize, Default, rmcp::schemars::JsonSchema)]
pub struct CollectItemsInput {
    /// Pickup radius in blocks (1-64). The bot walks toward dropped item
    /// entities within this radius.
    #[schemars(range(min = 1, max = 64))]
    pub radius: u32,
}

/// Handle `collect_items` MCP tool.
///
/// Validates `radius` is in `1..=64`, checks online status, then sends
/// [`BotCommand::CollectItems`]. The bot walks toward dropped item entities
/// within the radius; items are picked up automatically when the bot gets
/// close enough.
pub async fn handle_collect_items(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    input: CollectItemsInput,
) -> Result<String, BotError> {
    if input.radius < 1 || input.radius > 64 {
        return Err(BotError::InvalidParams(format!(
            "radius must be 1-64, got {}",
            input.radius
        )));
    }
    if !state.is_online() {
        return Err(BotError::Offline(
            "Bot is not connected to a server".to_string(),
        ));
    }

    let cmd = BotCommand::CollectItems(input.radius);
    match sender.send_command(cmd).await {
        Ok(result) => serde_json::to_string(&result)
            .map_err(|e| BotError::Internal(format!("Serialization error: {e}"))),
        Err(e) => Err(e),
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::create_command_channel;
    use crate::config::AppConfig;
    use serde_json::Value;

    fn setup() -> (Arc<SharedState>, BotCommandSender) {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, _receiver) = create_command_channel(4, Arc::clone(&state));
        (state, sender)
    }

    /// Create a channel where the receiver echoes back a successful BotResult.
    fn make_echo_channel() -> (Arc<SharedState>, BotCommandSender) {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, mut receiver) = create_command_channel(4, Arc::clone(&state));

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

    #[tokio::test]
    async fn test_switch_hotbar_slot_offline() {
        let (state, sender) = setup();
        let input = SwitchHotbarSlotInput { slot: 0 };
        let result = handle_switch_hotbar_slot(&state, &sender, input).await;
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    #[tokio::test]
    async fn test_switch_hotbar_slot_invalid() {
        let (state, sender) = setup();
        make_online(&state);
        let input = SwitchHotbarSlotInput { slot: 9 };
        let result = handle_switch_hotbar_slot(&state, &sender, input).await;
        assert!(
            matches!(result, Err(BotError::InvalidParams(ref msg)) if msg.contains("must be 0-8"))
        );
    }

    #[tokio::test]
    async fn test_switch_hotbar_slot_valid_range() {
        let (state, sender) = make_echo_channel();
        make_online(&state);
        for slot in 0..=8u8 {
            let result =
                handle_switch_hotbar_slot(&state, &sender, SwitchHotbarSlotInput { slot }).await;
            let _: Value = serde_json::from_str(&result.unwrap()).expect("valid JSON");
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
        assert!(matches!(result, Err(BotError::Offline(_))));
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
        assert!(
            matches!(result, Err(BotError::InvalidParams(ref msg)) if msg.contains("must be 0-35"))
        );
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
        assert!(
            matches!(result, Err(BotError::InvalidParams(ref msg)) if msg.contains("between 1 and 64"))
        );
    }

    /// Alias of `test_drop_item_zero_count` for spec clarity: the MCP layer
    /// must reject `count=0` with `BotError::InvalidParams` **before** any
    /// bot-side validation, so callers get a fast, accurate error.
    #[tokio::test]
    async fn test_drop_item_count_zero_rejected_at_mcp_layer() {
        let (state, sender) = setup();
        make_online(&state);
        let input = DropItemInput {
            slot: 5,
            count: Some(0),
        };
        let result = handle_drop_item(&state, &sender, input).await;
        match result {
            Err(BotError::InvalidParams(msg)) => {
                assert!(
                    msg.to_lowercase().contains("count"),
                    "msg should mention count, got: {msg}"
                );
            }
            other => panic!("expected InvalidParams for count=0, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_drop_item_default_count() {
        let (state, sender) = make_echo_channel();
        make_online(&state);
        let input = DropItemInput {
            slot: 5,
            count: None,
        };
        let result = handle_drop_item(&state, &sender, input).await;
        let _: Value = serde_json::from_str(&result.unwrap()).expect("valid JSON");
    }

    #[tokio::test]
    async fn test_use_item_offline() {
        let (state, sender) = setup();
        let input = UseItemInput { item_slot: None };
        let result = handle_use_item(&state, &sender, input).await;
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    #[tokio::test]
    async fn test_use_item_no_slot() {
        let (state, sender) = make_echo_channel();
        make_online(&state);
        let input = UseItemInput { item_slot: None };
        let result = handle_use_item(&state, &sender, input).await;
        let _: Value = serde_json::from_str(&result.unwrap()).expect("valid JSON");
    }

    #[tokio::test]
    async fn test_use_item_invalid_slot() {
        let (state, sender) = setup();
        make_online(&state);
        let input = UseItemInput {
            item_slot: Some(10),
        };
        let result = handle_use_item(&state, &sender, input).await;
        assert!(
            matches!(result, Err(BotError::InvalidParams(ref msg)) if msg.contains("must be 0-8"))
        );
    }

    #[tokio::test]
    async fn test_equip_tool_offline() {
        let (state, sender) = setup();
        let input = EquipToolInput {
            tool_type: "pickaxe".into(),
            material_preference: None,
        };
        let result = handle_equip_tool(&state, &sender, input).await;
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    #[tokio::test]
    async fn test_equip_tool_unknown_type() {
        let (state, sender) = setup();
        make_online(&state);
        let input = EquipToolInput {
            tool_type: "invalid_tool".into(),
            material_preference: None,
        };
        let result = handle_equip_tool(&state, &sender, input).await;
        assert!(
            matches!(result, Err(BotError::InvalidParams(ref msg)) if msg.contains("Unknown tool type"))
        );
    }

    #[tokio::test]
    async fn test_equip_tool_valid_types() {
        let (state, sender) = make_echo_channel();
        make_online(&state);
        for tt in ["pickaxe", "axe", "shovel", "sword", "shears", "hand"] {
            let input = EquipToolInput {
                tool_type: tt.into(),
                material_preference: None,
            };
            let result = handle_equip_tool(&state, &sender, input).await;
            let _: Value = serde_json::from_str(&result.unwrap()).expect("valid JSON");
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
        let result = handle_equip_tool(&state, &sender, input).await.unwrap();
        // The preference must be wired into an EquipToolWithMaterial command
        // (echoed back by the mock channel), not silently ignored.
        assert!(result.contains("EquipToolWithMaterial"));
        assert!(result.contains("Diamond"));
    }

    #[tokio::test]
    async fn test_equip_tool_invalid_material_preference() {
        let (state, sender) = make_echo_channel();
        make_online(&state);
        let input = EquipToolInput {
            tool_type: "pickaxe".into(),
            material_preference: Some("adamantium".into()),
        };
        let result = handle_equip_tool(&state, &sender, input).await;
        assert!(
            matches!(result, Err(BotError::InvalidParams(ref m)) if m.contains("material preference"))
        );
    }

    #[test]
    fn test_parse_material_tier() {
        assert_eq!(parse_material_tier("diamond"), Some(MaterialTier::Diamond));
        assert_eq!(
            parse_material_tier("NETHERITE"),
            Some(MaterialTier::Netherite)
        );
        assert_eq!(parse_material_tier("wooden"), Some(MaterialTier::Wood));
        assert_eq!(parse_material_tier("golden"), Some(MaterialTier::Gold));
        assert_eq!(parse_material_tier("unobtainium"), None);
    }

    #[test]
    fn test_parse_tool_type_all_variants() {
        assert_eq!(parse_tool_type("pickaxe"), Some(ToolType::Pickaxe));
        assert_eq!(parse_tool_type("axe"), Some(ToolType::Axe));
        assert_eq!(parse_tool_type("shovel"), Some(ToolType::Shovel));
        assert_eq!(parse_tool_type("hoe"), Some(ToolType::Hoe));
        assert_eq!(parse_tool_type("sword"), Some(ToolType::Sword));
        assert_eq!(parse_tool_type("shears"), Some(ToolType::Shears));
        assert_eq!(parse_tool_type("hand"), Some(ToolType::Hand));
    }

    #[test]
    fn test_parse_tool_type_case_insensitive() {
        assert_eq!(parse_tool_type("PICKAXE"), Some(ToolType::Pickaxe));
        assert_eq!(parse_tool_type("SWORD"), Some(ToolType::Sword));
        assert_eq!(parse_tool_type("HoE"), Some(ToolType::Hoe));
    }

    #[test]
    fn test_parse_tool_type_unknown() {
        assert_eq!(parse_tool_type("invalid"), None);
        assert_eq!(parse_tool_type(""), None);
    }

    // ── collect_items ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_collect_items_offline() {
        let (state, sender) = setup();
        let input = CollectItemsInput { radius: 8 };
        let result = handle_collect_items(&state, &sender, input).await;
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    #[tokio::test]
    async fn test_collect_items_invalid_radius_zero() {
        let (state, sender) = setup();
        make_online(&state);
        let input = CollectItemsInput { radius: 0 };
        let result = handle_collect_items(&state, &sender, input).await;
        assert!(matches!(result, Err(BotError::InvalidParams(ref msg))
                if msg.contains("radius must be 1-64") && msg.contains("got 0")));
    }

    #[tokio::test]
    async fn test_collect_items_invalid_radius_too_large() {
        let (state, sender) = setup();
        make_online(&state);
        let input = CollectItemsInput { radius: 65 };
        let result = handle_collect_items(&state, &sender, input).await;
        assert!(matches!(result, Err(BotError::InvalidParams(ref msg))
                if msg.contains("radius must be 1-64") && msg.contains("got 65")));
    }

    #[tokio::test]
    async fn test_collect_items_valid_radius() {
        let (state, sender) = make_echo_channel();
        make_online(&state);
        let input = CollectItemsInput { radius: 16 };
        let result = handle_collect_items(&state, &sender, input).await;
        let _: Value = serde_json::from_str(&result.unwrap()).expect("valid JSON");
    }

    #[tokio::test]
    async fn test_collect_items_sends_correct_command() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        state.set_online(true);
        let (sender, mut receiver) = create_command_channel(4, Arc::clone(&state));

        let responder = tokio::spawn(async move {
            let wrapped = receiver.recv().await.expect("should receive command");
            assert!(
                matches!(wrapped.command, BotCommand::CollectItems(16)),
                "expected CollectItems(16), got: {:?}",
                wrapped.command
            );
            wrapped
                .respond_to
                .send(Ok(crate::types::BotResult {
                    success: true,
                    message: "collected 3 items".into(),
                    data: None,
                }))
                .expect("should respond");
        });

        let input = CollectItemsInput { radius: 16 };
        let result = handle_collect_items(&state, &sender, input).await.unwrap();
        let json: Value = serde_json::from_str(&result).expect("valid JSON");
        assert!(
            json.get("success")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        );
        responder.await.expect("responder should finish");
    }
}
