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
    crate::mcp::common::require_online(state)?;

    let cmd = BotCommand::SwitchHotbarSlot(input.slot);
    crate::mcp::common::send_and_serialize(sender, cmd).await
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
    if !(1..=64).contains(&count) {
        return Err(BotError::InvalidParams(format!(
            "Count must be between 1 and 64, got {count}"
        )));
    }
    crate::mcp::common::require_online(state)?;

    let cmd = BotCommand::DropItem(input.slot, count);
    crate::mcp::common::send_and_serialize(sender, cmd).await
}

// ── set_hotbar_item ─────────────────────────────────────────────────────────

/// Input for the `set_hotbar_item` MCP tool.
#[derive(Deserialize, Default, rmcp::schemars::JsonSchema)]
pub struct SetHotbarItemInput {
    /// Hotbar slot to place the item into (0-8).
    #[schemars(range(min = 0, max = 8))]
    pub hotbar_slot: u8,
    /// Item id to move into the hotbar (e.g. "dirt", "iron_sword").
    ///
    /// The item must already exist in the bot's inventory — this tool swaps
    /// an existing stack into the hotbar and cannot conjure items (that still
    /// requires an `/give`-style command).
    pub item_id: String,
    /// Minimum stack size required in the source slot (default 1).
    #[schemars(range(min = 1, max = 64))]
    pub count: Option<u8>,
}

/// Handle `set_hotbar_item` MCP tool.
///
/// Moves the first inventory slot holding at least `count` of `item_id` into
/// hotbar slot `hotbar_slot` via a container swap-click (the in-game "press
/// the hotbar number while clicking a slot" operation). No server-side command
/// is involved, so it is reliable where `/item replace` syntax differs across
/// servers. Fails with `InvalidParams` when the item is not in the inventory.
pub async fn handle_set_hotbar_item(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    input: SetHotbarItemInput,
) -> Result<String, BotError> {
    if input.hotbar_slot > 8 {
        return Err(BotError::InvalidParams(format!(
            "hotbar_slot must be 0-8, got {}",
            input.hotbar_slot
        )));
    }
    if input.item_id.trim().is_empty() {
        return Err(BotError::InvalidParams(
            "item_id cannot be empty".to_string(),
        ));
    }
    let count = input.count.unwrap_or(1);
    if !(1..=64).contains(&count) {
        return Err(BotError::InvalidParams(format!(
            "count must be between 1 and 64, got {count}"
        )));
    }
    crate::mcp::common::require_online(state)?;

    let cmd = BotCommand::MoveItemToHotbar(input.hotbar_slot, input.item_id, count);
    crate::mcp::common::send_and_serialize(sender, cmd).await
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
    crate::mcp::common::require_online(state)?;

    // When a slot is requested, send a single atomic UseItemWithSlot command
    // so the switch + use cannot be interleaved with other commands under
    // HTTP transport concurrency. Without a slot, fall back to UseItem.
    let cmd = match input.item_slot {
        Some(slot) => BotCommand::UseItemWithSlot(slot),
        None => BotCommand::UseItem,
    };
    crate::mcp::common::send_and_serialize(sender, cmd).await
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

    crate::mcp::common::require_online(state)?;

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
    crate::mcp::common::require_online(state)?;

    // Force a snapshot refresh first so the executor sees dropped item
    // entities that arrived after the last 500 ms-throttled snapshot — a
    // just-dropped item would otherwise be invisible to `collect_items`
    // ("No items to collect" right after a drop).
    crate::mcp::tools_query::refresh_snapshot_and_wait(state).await;

    let cmd = BotCommand::CollectItems(input.radius);
    crate::mcp::common::send_and_serialize(sender, cmd).await
}

// ── give_item ───────────────────────────────────────────────────────────────

/// Input for the `give_item` MCP tool.
#[derive(Deserialize, Default, rmcp::schemars::JsonSchema)]
pub struct GiveItemInput {
    /// Item id to give, e.g. "diamond_pickaxe" or "minecraft:diamond_pickaxe".
    pub item_id: String,
    /// Stack size to give (1-64, default 1).
    #[schemars(range(min = 1, max = 64))]
    pub count: Option<u8>,
    /// Where to put the item: "inventory" (default) or "hotbar".
    pub target: Option<String>,
    /// Hotbar slot to fill when target is "hotbar" (0-8, default 0).
    #[schemars(range(min = 0, max = 8))]
    pub hotbar_slot: Option<u8>,
}

/// Handle `give_item` MCP tool.
///
/// Gives the bot an item by running `/give <bot> <item> <count>`; when
/// `target` is "hotbar" it follows with
/// `/item replace entity <bot> hotbar.<slot> with <item> <count>` and, if the
/// server rejects `/item replace`, falls back to the swap-click
/// [`BotCommand::MoveItemToHotbar`] path (reliable wherever the item already
/// landed in the inventory). A rejection of the initial `/give` (for example
/// an unknown item id) is propagated as [`BotError::CommandRejected`] instead
/// of a fake success, and a `success:false` `/give` result is returned
/// verbatim instead of fabricating "Gave ...". Requires server commands
/// (op) — command availability is verified live via a `/seed` probe, and
/// only a probe-confirmed rejection yields `PermissionDenied` (the cached
/// snapshot's `commands_enabled` can be a stale `PermissionLevel` heuristic
/// right after a reconnect).
pub async fn handle_give_item(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    input: GiveItemInput,
) -> Result<String, BotError> {
    if input.item_id.trim().is_empty() {
        return Err(BotError::InvalidParams(
            "item_id cannot be empty".to_string(),
        ));
    }
    let count = input.count.unwrap_or(1);
    if !(1..=64).contains(&count) {
        return Err(BotError::InvalidParams(format!(
            "count must be between 1 and 64, got {count}"
        )));
    }
    let target_hotbar = match input.target.as_deref().map(|t| t.to_lowercase()) {
        None => false,
        Some(t) if t == "hotbar" => true,
        Some(t) if t == "inventory" => false,
        Some(other) => {
            return Err(BotError::InvalidParams(format!(
                "target must be 'inventory' or 'hotbar', got '{other}'"
            )));
        }
    };
    let hotbar_slot = input.hotbar_slot.unwrap_or(0);
    if hotbar_slot > 8 {
        return Err(BotError::InvalidParams(format!(
            "hotbar_slot must be 0-8, got {hotbar_slot}"
        )));
    }
    crate::mcp::common::require_online(state)?;
    // Commands-availability gate. The cached snapshot's `commands_enabled`
    // may reflect the `PermissionLevel` heuristic, which lags the real
    // server state right after a reconnect (the permission component reads
    // 0 before the first sync) — trusting it would reject /give for a bot
    // that can actually run commands. Re-probe live via /seed: only reject
    // when the probe itself confirms commands are unavailable. When the
    // probe is unknown the /give attempt proceeds and a real rejection is
    // surfaced by `handle_execute_command` (Bug-1 fix) instead.
    if let Some(probe) = crate::mcp::tools_query::probe_commands_enabled(state, sender).await
        && !probe
    {
        return Err(BotError::PermissionDenied(
            "Server commands are disabled for this bot — give_item needs OP permissions (verified via /seed probe)"
                .to_string(),
        ));
    }
    // Minecraft command ids are namespaced; the MCP contract uses bare
    // snake_case ids. Add the prefix unless already namespaced.
    let namespaced = if input.item_id.contains(':') {
        input.item_id.clone()
    } else {
        format!("minecraft:{}", input.item_id)
    };
    let username = {
        let snapshot = state.read_snapshot();
        let name = snapshot.self_player.username.clone();
        if name.is_empty() {
            return Err(BotError::Internal(
                "Cannot determine the bot's username for /give".to_string(),
            ));
        }
        name
    };

    let give_cmd = BotCommand::ExecuteCommand(format!("/give {username} {namespaced} {count}"));
    // Bind the result instead of dropping it (L-6): the executor can report
    // `success:false` without an `Err` (e.g. server feedback indicates the
    // give was not delivered). Fabricating "Gave ..." would be a fake
    // success — return the honest BotResult verbatim instead.
    let give_result = sender.send_command(give_cmd).await?;
    if !give_result.success {
        return serde_json::to_string(&give_result)
            .map_err(|e| BotError::Internal(format!("Serialization error: {e}")));
    }

    if target_hotbar {
        let replace_cmd = BotCommand::ExecuteCommand(format!(
            "/item replace entity {username} hotbar.{hotbar_slot} with {namespaced} {count}"
        ));
        match sender.send_command(replace_cmd).await {
            Ok(_) => {
                return serde_json::to_string(&crate::types::BotResult {
                    success: true,
                    message: format!(
                        "Gave {count}x {namespaced} to {username} into hotbar slot {hotbar_slot} (/item replace)"
                    ),
                    data: Some(serde_json::json!({
                        "item_id": input.item_id,
                        "count": count,
                        "target": "hotbar",
                        "hotbar_slot": hotbar_slot,
                        "method": "item_replace",
                    })),
                })
                .map_err(|e| BotError::Internal(format!("Serialization error: {e}")));
            }
            Err(BotError::CommandRejected { .. }) => {
                // `/item replace` is not supported on every server. The item
                // is already in the inventory (the /give succeeded), so fall
                // back to the reliable swap-click move. The inventory stores
                // BARE ids ("cobblestone") while the input may carry a
                // "minecraft:" namespace (valid in commands) — strip it so
                // the MoveItemToHotbar match finds the stack (L-7).
                let bare_id = input
                    .item_id
                    .strip_prefix("minecraft:")
                    .unwrap_or(&input.item_id)
                    .to_string();
                let swap = BotCommand::MoveItemToHotbar(hotbar_slot, bare_id, count);
                match sender.send_command(swap).await {
                    Ok(swap_result) => {
                        return serde_json::to_string(&crate::types::BotResult {
                            success: swap_result.success,
                            message: format!(
                                "Gave {count}x {namespaced} to {username}; /item replace rejected, moved into hotbar slot {hotbar_slot} via swap-click"
                            ),
                            data: Some(serde_json::json!({
                                "item_id": input.item_id,
                                "count": count,
                                "target": "hotbar",
                                "hotbar_slot": hotbar_slot,
                                "method": "swap_click",
                            })),
                        })
                        .map_err(|e| BotError::Internal(format!("Serialization error: {e}")));
                    }
                    Err(e) => return Err(e),
                }
            }
            Err(e) => return Err(e),
        }
    }

    serde_json::to_string(&crate::types::BotResult {
        success: true,
        message: format!("Gave {count}x {namespaced} to {username} (inventory)"),
        data: Some(serde_json::json!({
            "item_id": input.item_id,
            "count": count,
            "target": "inventory",
            "method": "give",
        })),
    })
    .map_err(|e| BotError::Internal(format!("Serialization error: {e}")))
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

    // -- set_hotbar_item -------------------------------------------------------

    #[tokio::test]
    async fn test_set_hotbar_item_offline() {
        let (state, sender) = setup();
        let input = SetHotbarItemInput {
            hotbar_slot: 0,
            item_id: "dirt".into(),
            count: Some(1),
        };
        let result = handle_set_hotbar_item(&state, &sender, input).await;
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    #[tokio::test]
    async fn test_set_hotbar_item_invalid_slot() {
        let (state, sender) = setup();
        make_online(&state);
        let input = SetHotbarItemInput {
            hotbar_slot: 9,
            item_id: "dirt".into(),
            count: Some(1),
        };
        let result = handle_set_hotbar_item(&state, &sender, input).await;
        assert!(matches!(result, Err(BotError::InvalidParams(ref msg)) if msg.contains("0-8")));
    }

    #[tokio::test]
    async fn test_set_hotbar_item_empty_item_id() {
        let (state, sender) = setup();
        make_online(&state);
        let input = SetHotbarItemInput {
            hotbar_slot: 0,
            item_id: "  ".into(),
            count: Some(1),
        };
        let result = handle_set_hotbar_item(&state, &sender, input).await;
        assert!(matches!(result, Err(BotError::InvalidParams(ref msg)) if msg.contains("item_id")));
    }

    #[tokio::test]
    async fn test_set_hotbar_item_invalid_count() {
        let (state, sender) = setup();
        make_online(&state);
        let input = SetHotbarItemInput {
            hotbar_slot: 0,
            item_id: "dirt".into(),
            count: Some(0),
        };
        let result = handle_set_hotbar_item(&state, &sender, input).await;
        assert!(matches!(result, Err(BotError::InvalidParams(ref msg)) if msg.contains("count")));
    }

    #[tokio::test]
    async fn test_set_hotbar_item_sends_move_command() {
        let (state, sender) = make_echo_channel();
        make_online(&state);
        let input = SetHotbarItemInput {
            hotbar_slot: 3,
            item_id: "iron_sword".into(),
            count: Some(2),
        };
        let result = handle_set_hotbar_item(&state, &sender, input).await;
        let value: Value = serde_json::from_str(&result.unwrap()).expect("valid JSON");
        assert_eq!(value["success"], true);
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

    // ── give_item ──────────────────────────────────────────────────────────

    /// Online state with username TestBot and commands enabled so give_item
    /// passes its gates and the emitted commands can be asserted.
    fn make_give_state() -> Arc<SharedState> {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        state.set_online(true);
        state.update_snapshot(crate::types::WorldSnapshot {
            self_player: crate::types::SelfPlayer {
                username: "TestBot".into(),
                ..Default::default()
            },
            commands_enabled: Some(true),
            ..Default::default()
        });
        state
    }

    /// Consume the `/seed` command-availability probe that `give_item` sends
    /// first and reply success. The probe runs before every /give so the
    /// commands-enabled gate reflects live server state rather than the
    /// possibly-stale snapshot heuristic.
    async fn consume_probe(receiver: &mut crate::channel::BotCommandReceiver) {
        let probe = receiver.recv().await.expect("probe command");
        assert!(
            matches!(probe.command, BotCommand::ExecuteCommand(ref c) if c == "/seed"),
            "first command must be the /seed probe, got: {:?}",
            probe.command
        );
        probe
            .respond_to
            .send(Ok(crate::types::BotResult {
                success: true,
                message: "ok".into(),
                data: None,
            }))
            .unwrap();
    }

    #[tokio::test]
    async fn test_give_item_to_inventory_sends_give_command() {
        let state = make_give_state();
        let (sender, mut receiver) = create_command_channel(4, Arc::clone(&state));

        let responder = tokio::spawn(async move {
            consume_probe(&mut receiver).await;
            let wrapped = receiver.recv().await.expect("should receive give");
            assert!(
                matches!(
                    wrapped.command,
                    BotCommand::ExecuteCommand(ref c)
                        if c == "/give TestBot minecraft:diamond_pickaxe 1"
                ),
                "got: {:?}",
                wrapped.command
            );
            wrapped
                .respond_to
                .send(Ok(crate::types::BotResult {
                    success: true,
                    message: "ok".into(),
                    data: None,
                }))
                .expect("respond");
        });

        let input = GiveItemInput {
            item_id: "diamond_pickaxe".into(),
            count: None,
            target: None,
            hotbar_slot: None,
        };
        let result = handle_give_item(&state, &sender, input).await.unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["success"], true);
        assert_eq!(parsed["data"]["target"], "inventory");
        assert_eq!(parsed["data"]["method"], "give");
        responder.await.expect("responder finished");
    }

    #[tokio::test]
    async fn test_give_item_inventory_command_rejected_propagates() {
        // Regression: /give for an unknown item id must surface
        // CommandRejected (from the executor's rejection scan) instead of the
        // fake "Gave N x nonexistent_item" success observed in the field.
        let state = make_give_state();
        let (sender, mut receiver) = create_command_channel(4, Arc::clone(&state));

        let responder = tokio::spawn(async move {
            consume_probe(&mut receiver).await;
            let give = receiver.recv().await.expect("give command");
            assert!(matches!(
                give.command,
                BotCommand::ExecuteCommand(ref c)
                    if c == "/give TestBot minecraft:nonexistent_item_xyz 1"
            ));
            give.respond_to
                .send(Err(BotError::CommandRejected {
                    command: "/give TestBot minecraft:nonexistent_item_xyz 1".into(),
                    feedback: "Unknown item 'minecraft:nonexistent_item_xyz'".into(),
                }))
                .unwrap();
        });

        let input = GiveItemInput {
            item_id: "nonexistent_item_xyz".into(),
            count: None,
            target: None,
            hotbar_slot: None,
        };
        let result = handle_give_item(&state, &sender, input).await;
        assert!(
            matches!(result, Err(BotError::CommandRejected { ref feedback, .. }) if feedback.contains("Unknown item")),
            "give_item must propagate the /give rejection, got: {result:?}"
        );
        responder.await.expect("responder finished");
    }

    /// RED (L-6): the executor can report `Ok(BotResult{success:false})`
    /// without an Err (e.g. server feedback indicates the give was not
    /// delivered). give_item must not fabricate a "Gave ..." success — it
    /// returns the honest BotResult (message/data verbatim) instead.
    #[tokio::test]
    async fn test_give_item_reports_executor_failure() {
        let state = make_give_state();
        let (sender, mut receiver) = create_command_channel(4, Arc::clone(&state));

        let responder = tokio::spawn(async move {
            consume_probe(&mut receiver).await;
            let give = receiver.recv().await.expect("give command");
            give.respond_to
                .send(Ok(crate::types::BotResult {
                    success: false,
                    message: "server said no".into(),
                    data: Some(serde_json::json!({ "reason": "give_undelivered" })),
                }))
                .unwrap();
        });

        let input = GiveItemInput {
            item_id: "dirt".into(),
            count: None,
            target: None,
            hotbar_slot: None,
        };
        let result = handle_give_item(&state, &sender, input)
            .await
            .expect("executor failure must be returned, not errored");
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(
            parsed["success"], false,
            "must not claim success when the executor failed: {result}"
        );
        assert_eq!(parsed["message"], "server said no");
        assert_eq!(parsed["data"]["reason"], "give_undelivered");
        responder.await.expect("responder finished");
    }

    #[tokio::test]
    async fn test_give_item_to_hotbar_sends_item_replace() {
        let state = make_give_state();
        let (sender, mut receiver) = create_command_channel(4, Arc::clone(&state));

        let responder = tokio::spawn(async move {
            consume_probe(&mut receiver).await;
            let first = receiver.recv().await.expect("give");
            assert!(matches!(
                first.command,
                BotCommand::ExecuteCommand(ref c) if c == "/give TestBot minecraft:water_bucket 1"
            ));
            first
                .respond_to
                .send(Ok(crate::types::BotResult {
                    success: true,
                    message: "ok".into(),
                    data: None,
                }))
                .unwrap();
            let second = receiver.recv().await.expect("replace");
            assert!(matches!(
                second.command,
                BotCommand::ExecuteCommand(ref c)
                    if c == "/item replace entity TestBot hotbar.3 with minecraft:water_bucket 1"
            ));
            second
                .respond_to
                .send(Ok(crate::types::BotResult {
                    success: true,
                    message: "ok".into(),
                    data: None,
                }))
                .unwrap();
        });

        let input = GiveItemInput {
            item_id: "water_bucket".into(),
            count: None,
            target: Some("hotbar".into()),
            hotbar_slot: Some(3),
        };
        let result = handle_give_item(&state, &sender, input).await.unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["data"]["method"], "item_replace");
        assert_eq!(parsed["data"]["hotbar_slot"], 3);
        responder.await.expect("responder finished");
    }

    #[tokio::test]
    async fn test_give_item_falls_back_to_swap_click_when_replace_rejected() {
        let state = make_give_state();
        let (sender, mut receiver) = create_command_channel(4, Arc::clone(&state));

        let responder = tokio::spawn(async move {
            consume_probe(&mut receiver).await;
            let first = receiver.recv().await.expect("give");
            first
                .respond_to
                .send(Ok(crate::types::BotResult {
                    success: true,
                    message: "ok".into(),
                    data: None,
                }))
                .unwrap();
            let second = receiver.recv().await.expect("replace");
            second
                .respond_to
                .send(Err(BotError::CommandRejected {
                    command: "/item replace ...".into(),
                    feedback: "Unknown command".into(),
                }))
                .unwrap();
            let third = receiver.recv().await.expect("swap fallback");
            assert!(
                matches!(
                    third.command,
                    BotCommand::MoveItemToHotbar(3, ref id, 1) if id == "water_bucket"
                ),
                "got: {:?}",
                third.command
            );
            third
                .respond_to
                .send(Ok(crate::types::BotResult {
                    success: true,
                    message: "ok".into(),
                    data: None,
                }))
                .unwrap();
        });

        let input = GiveItemInput {
            item_id: "water_bucket".into(),
            count: None,
            target: Some("hotbar".into()),
            hotbar_slot: Some(3),
        };
        let result = handle_give_item(&state, &sender, input).await.unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["data"]["method"], "swap_click");
        responder.await.expect("responder finished");
    }

    /// RED (L-7): with a namespaced item_id the /give succeeds (namespaced
    /// ids are valid in commands) but the swap fallback previously passed the
    /// raw id to `MoveItemToHotbar`, which matches against the inventory's
    /// BARE ids — "minecraft:cobblestone" never matched and the tool reported
    /// InvalidParams for an item that exists. The fallback must strip the
    /// "minecraft:" prefix before the inventory match.
    #[tokio::test]
    async fn test_give_item_namespaced_swap_fallback_strips_prefix() {
        let state = make_give_state();
        let (sender, mut receiver) = create_command_channel(4, Arc::clone(&state));

        let responder = tokio::spawn(async move {
            consume_probe(&mut receiver).await;
            let first = receiver.recv().await.expect("give");
            first
                .respond_to
                .send(Ok(crate::types::BotResult {
                    success: true,
                    message: "ok".into(),
                    data: None,
                }))
                .unwrap();
            let second = receiver.recv().await.expect("replace");
            second
                .respond_to
                .send(Err(BotError::CommandRejected {
                    command: "/item replace ...".into(),
                    feedback: "Unknown command".into(),
                }))
                .unwrap();
            let third = receiver.recv().await.expect("swap fallback");
            assert!(
                matches!(
                    third.command,
                    BotCommand::MoveItemToHotbar(0, ref id, 1) if id == "cobblestone"
                ),
                "namespaced id must be stripped for the bare-id inventory match, got: {:?}",
                third.command
            );
            third
                .respond_to
                .send(Ok(crate::types::BotResult {
                    success: true,
                    message: "ok".into(),
                    data: None,
                }))
                .unwrap();
        });

        let input = GiveItemInput {
            item_id: "minecraft:cobblestone".into(),
            count: None,
            target: Some("hotbar".into()),
            hotbar_slot: None,
        };
        let result = handle_give_item(&state, &sender, input).await.unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["data"]["method"], "swap_click");
        responder.await.expect("responder finished");
    }

    #[tokio::test]
    async fn test_give_item_rejected_when_commands_disabled() {
        // The gate must reject only when the LIVE /seed probe confirms
        // commands are unavailable — a rejected probe wins over whatever the
        // snapshot heuristic cached.
        let state = Arc::new(SharedState::new(AppConfig::default()));
        state.set_online(true);
        state.update_snapshot(crate::types::WorldSnapshot {
            commands_enabled: Some(false),
            ..Default::default()
        });
        let (sender, mut receiver) = create_command_channel(4, Arc::clone(&state));

        let responder = tokio::spawn(async move {
            let probe = receiver.recv().await.expect("probe command");
            assert!(matches!(
                probe.command,
                BotCommand::ExecuteCommand(ref c) if c == "/seed"
            ));
            probe
                .respond_to
                .send(Err(BotError::CommandRejected {
                    command: "/seed".into(),
                    feedback: "You do not have permission to use this command".into(),
                }))
                .unwrap();
        });

        let input = GiveItemInput {
            item_id: "dirt".into(),
            count: None,
            target: None,
            hotbar_slot: None,
        };
        let result = handle_give_item(&state, &sender, input).await;
        assert!(
            matches!(result, Err(BotError::PermissionDenied(ref msg)) if msg.contains("verified via /seed probe")),
            "probe-confirmed disabled must yield PermissionDenied, got: {result:?}"
        );
        responder.await.expect("responder finished");
    }

    #[tokio::test]
    async fn test_give_item_probe_overrides_stale_snapshot_disabled() {
        // Regression for the intermittent false "Permission denied" during
        // the functional test: the cached snapshot reported
        // `commands_enabled: Some(false)` from the PermissionLevel heuristic,
        // but the live probe (and the actual /give) succeeded. The gate must
        // trust the probe, not the stale snapshot.
        let state = Arc::new(SharedState::new(AppConfig::default()));
        state.set_online(true);
        state.update_snapshot(crate::types::WorldSnapshot {
            self_player: crate::types::SelfPlayer {
                username: "TestBot".into(),
                ..Default::default()
            },
            commands_enabled: Some(false),
            ..Default::default()
        });
        let (sender, mut receiver) = create_command_channel(4, Arc::clone(&state));

        let responder = tokio::spawn(async move {
            consume_probe(&mut receiver).await;
            let give = receiver.recv().await.expect("give command");
            assert!(matches!(
                give.command,
                BotCommand::ExecuteCommand(ref c) if c == "/give TestBot minecraft:dirt 1"
            ));
            give.respond_to
                .send(Ok(crate::types::BotResult {
                    success: true,
                    message: "ok".into(),
                    data: None,
                }))
                .unwrap();
        });

        let input = GiveItemInput {
            item_id: "dirt".into(),
            count: None,
            target: None,
            hotbar_slot: None,
        };
        let result = handle_give_item(&state, &sender, input)
            .await
            .expect("stale snapshot must not block a working /give");
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["success"], true);
        assert_eq!(parsed["data"]["method"], "give");
        // The successful probe is now cached, matching the real server state.
        assert_eq!(state.get_commands_probe(), Some(true));
        responder.await.expect("responder finished");
    }

    #[tokio::test]
    async fn test_give_item_validation_errors() {
        let (state, sender) = setup();
        make_online(&state);

        let empty = GiveItemInput {
            item_id: "  ".into(),
            count: None,
            target: None,
            hotbar_slot: None,
        };
        assert!(matches!(
            handle_give_item(&state, &sender, empty).await,
            Err(BotError::InvalidParams(_))
        ));

        let bad_count = GiveItemInput {
            item_id: "dirt".into(),
            count: Some(0),
            target: None,
            hotbar_slot: None,
        };
        assert!(matches!(
            handle_give_item(&state, &sender, bad_count).await,
            Err(BotError::InvalidParams(_))
        ));

        let bad_target = GiveItemInput {
            item_id: "dirt".into(),
            count: None,
            target: Some("backpack".into()),
            hotbar_slot: None,
        };
        assert!(matches!(
            handle_give_item(&state, &sender, bad_target).await,
            Err(BotError::InvalidParams(_))
        ));

        let bad_slot = GiveItemInput {
            item_id: "dirt".into(),
            count: None,
            target: Some("hotbar".into()),
            hotbar_slot: Some(9),
        };
        assert!(matches!(
            handle_give_item(&state, &sender, bad_slot).await,
            Err(BotError::InvalidParams(_))
        ));
    }
}
