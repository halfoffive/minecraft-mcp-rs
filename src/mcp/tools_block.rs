//! MCP tools for block operations (break, place, use item on block).
//!
//! Each tool validates coordinates through [`validate_block_pos`], checks
//! online status, and dispatches a [`BotCommand`] through the bot command
//! channel.

use std::sync::Arc;

use serde::Deserialize;

use crate::channel::BotCommandSender;
use crate::command_validate::validate_block_pos;
use crate::error::BotError;
use crate::state::SharedState;
use crate::types::{ActAction, BlockPos, BotCommand};

// ── Tool descriptions (with Creative-mode hint) ───────────────────────────

/// Hint appended to block-tool descriptions recommending Creative-mode
/// alternatives for bulk building.
pub const CREATIVE_MODE_HINT: &str =
    "In Creative mode, prefer `execute_command` with `/fill` or `/setblock` for bulk building.";

/// Full description for the `break_block` MCP tool.
pub const BREAK_BLOCK_DESCRIPTION: &str = "Break a block at the given position. If use_best_tool is true, runs the full compound mine flow (tool selection, movement, mining, verification) equivalent to act(Mine). In Creative mode, prefer `execute_command` with `/fill` or `/setblock` for bulk building.";

/// Full description for the `place_block` MCP tool.
pub const PLACE_BLOCK_DESCRIPTION: &str = "Place a block at the given position. In Creative mode, prefer `execute_command` with `/fill` or `/setblock` for bulk building.";

// ── break_block ────────────────────────────────────────────────────────────

/// Input for the `break_block` MCP tool.
#[derive(Deserialize, Default, rmcp::schemars::JsonSchema)]
pub struct BreakBlockInput {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub use_best_tool: Option<bool>,
}

/// Handle `break_block` MCP tool.
///
/// Validates coordinates, checks online status, then sends either
/// [`BotCommand::BreakBlock`] (raw) or `BotCommand::Act(ActAction::Mine)`
/// (full compound mine flow) depending on `use_best_tool`. When
/// `use_best_tool` is `true`, the behavior is equivalent to `act(Mine)`.
///
/// In Creative mode, prefer `execute_command` with `/fill` or `/setblock` for bulk building.
pub async fn handle_break_block(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    input: BreakBlockInput,
) -> Result<String, BotError> {
    // Validate coordinates are within world bounds
    if let Err(e) = validate_block_pos(&BlockPos::new(input.x, input.y, input.z)) {
        return Err(BotError::InvalidParams(e));
    }

    if !state.is_online() {
        return Err(BotError::Offline(
            "Bot is not connected to a server".to_string(),
        ));
    }

    let pos = BlockPos::new(input.x, input.y, input.z);
    let cmd = if input.use_best_tool == Some(true) {
        BotCommand::Act(ActAction::Mine { block_pos: pos }, None)
    } else {
        BotCommand::BreakBlock(pos)
    };
    match sender.send_command(cmd).await {
        Ok(result) => serde_json::to_string(&result)
            .map_err(|e| BotError::Internal(format!("Serialization error: {e}"))),
        Err(e) => Err(e),
    }
}

// ── place_block ────────────────────────────────────────────────────────────

/// Input for the `place_block` MCP tool.
#[derive(Deserialize, Default, rmcp::schemars::JsonSchema)]
pub struct PlaceBlockInput {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    /// Hotbar slot holding the block to place (0-8).
    #[schemars(range(min = 0, max = 8))]
    pub item_slot: u8,
}

/// Handle `place_block` MCP tool.
///
/// Validates coordinates and slot, checks online status, then sends
/// [`BotCommand::PlaceBlock`] with the target position. The `item_slot`
/// is encoded as `"slot:N"` in the block type field so the bot executor
/// can resolve the actual block type from the player's inventory.
///
/// In Creative mode, prefer `execute_command` with `/fill` or `/setblock` for bulk building.
pub async fn handle_place_block(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    input: PlaceBlockInput,
) -> Result<String, BotError> {
    // Validate coordinates are within world bounds
    if let Err(e) = validate_block_pos(&BlockPos::new(input.x, input.y, input.z)) {
        return Err(BotError::InvalidParams(e));
    }

    // Validate item slot is in hotbar range
    if input.item_slot > 8 {
        return Err(BotError::InvalidParams(format!(
            "item_slot must be 0-8, got {}",
            input.item_slot
        )));
    }

    if !state.is_online() {
        return Err(BotError::Offline(
            "Bot is not connected to a server".to_string(),
        ));
    }

    let cmd = BotCommand::PlaceBlock(
        BlockPos::new(input.x, input.y, input.z),
        format!("slot:{}", input.item_slot),
    );
    match sender.send_command(cmd).await {
        Ok(result) => serde_json::to_string(&result)
            .map_err(|e| BotError::Internal(format!("Serialization error: {e}"))),
        Err(e) => Err(e),
    }
}

// ── use_item_on_block ──────────────────────────────────────────────────────

/// Input for the `use_item_on_block` MCP tool.
#[derive(Deserialize, Default, rmcp::schemars::JsonSchema)]
pub struct UseItemOnBlockInput {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    /// Hotbar slot holding the item to use (0-8). `None` keeps the currently
    /// held item.
    #[schemars(range(min = 0, max = 8))]
    pub item_slot: Option<u8>,
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
) -> Result<String, BotError> {
    // Validate coordinates are within world bounds
    if let Err(e) = validate_block_pos(&BlockPos::new(input.x, input.y, input.z)) {
        return Err(BotError::InvalidParams(e));
    }

    // Validate optional item slot
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

    let cmd = BotCommand::UseItemOnBlock(BlockPos::new(input.x, input.y, input.z), input.item_slot);
    match sender.send_command(cmd).await {
        Ok(result) => serde_json::to_string(&result)
            .map_err(|e| BotError::Internal(format!("Serialization error: {e}"))),
        Err(e) => Err(e),
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;
    use crate::channel::create_command_channel;
    use crate::config::AppConfig;

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
        assert!(matches!(result, Err(BotError::Offline(_))));
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
        assert!(
            matches!(result, Err(BotError::InvalidParams(ref msg)) if msg.contains("out of bounds") || msg.contains("out of range"))
        );
    }

    #[tokio::test]
    async fn test_break_block_valid() {
        let (state, sender) = make_echo_channel();
        make_online(&state);
        let input = BreakBlockInput {
            x: 10,
            y: 64,
            z: -5,
            use_best_tool: None,
        };
        let result = handle_break_block(&state, &sender, input).await.unwrap();
        let _: Value = serde_json::from_str(&result).expect("valid JSON");
    }

    #[tokio::test]
    async fn test_break_block_with_best_tool() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        make_online(&state);
        let (sender, mut receiver) = create_command_channel(4, Arc::clone(&state));

        let responder = tokio::spawn(async move {
            let wrapped = receiver.recv().await.expect("should receive command");
            assert!(
                matches!(
                    wrapped.command,
                    BotCommand::Act(ActAction::Mine { block_pos }, _)
                        if block_pos == BlockPos::new(10, 64, -5)
                ),
                "expected Act(Mine) for use_best_tool=true, got: {:?}",
                wrapped.command
            );
            wrapped
                .respond_to
                .send(Ok(crate::types::BotResult {
                    success: true,
                    message: "mining".into(),
                    data: None,
                }))
                .expect("should respond");
        });

        let input = BreakBlockInput {
            x: 10,
            y: 64,
            z: -5,
            use_best_tool: Some(true),
        };
        let result = handle_break_block(&state, &sender, input).await.unwrap();
        let _: Value = serde_json::from_str(&result).expect("valid JSON");
        responder.await.expect("responder should finish");
    }

    #[tokio::test]
    async fn test_break_block_with_best_tool_false() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        make_online(&state);
        let (sender, mut receiver) = create_command_channel(4, Arc::clone(&state));

        let responder = tokio::spawn(async move {
            let wrapped = receiver.recv().await.expect("should receive command");
            assert!(
                matches!(
                    wrapped.command,
                    BotCommand::BreakBlock(pos) if pos == BlockPos::new(10, 64, -5)
                ),
                "expected BreakBlock for use_best_tool=false, got: {:?}",
                wrapped.command
            );
            wrapped
                .respond_to
                .send(Ok(crate::types::BotResult {
                    success: true,
                    message: "broken".into(),
                    data: None,
                }))
                .expect("should respond");
        });

        let input = BreakBlockInput {
            x: 10,
            y: 64,
            z: -5,
            use_best_tool: Some(false),
        };
        let result = handle_break_block(&state, &sender, input).await.unwrap();
        let _: Value = serde_json::from_str(&result).expect("valid JSON");
        responder.await.expect("responder should finish");
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
        assert!(matches!(result, Err(BotError::Offline(_))));
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
        assert!(
            matches!(result, Err(BotError::InvalidParams(ref msg)) if msg.contains("out of bounds") || msg.contains("out of range"))
        );
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
        assert!(
            matches!(result, Err(BotError::InvalidParams(ref msg)) if msg.contains("must be 0-8"))
        );
    }

    #[tokio::test]
    async fn test_place_block_valid() {
        let (state, sender) = make_echo_channel();
        make_online(&state);
        let input = PlaceBlockInput {
            x: 5,
            y: 65,
            z: 10,
            item_slot: 3,
        };
        let result = handle_place_block(&state, &sender, input).await.unwrap();
        let _: Value = serde_json::from_str(&result).expect("valid JSON");
    }

    #[tokio::test]
    async fn test_place_block_min_slot_valid() {
        let (state, sender) = make_echo_channel();
        make_online(&state);
        let input = PlaceBlockInput {
            x: 0,
            y: 64,
            z: 0,
            item_slot: 0,
        };
        let result = handle_place_block(&state, &sender, input).await.unwrap();
        let _: Value = serde_json::from_str(&result).expect("valid JSON");
    }

    #[tokio::test]
    async fn test_place_block_max_slot_valid() {
        let (state, sender) = make_echo_channel();
        make_online(&state);
        let input = PlaceBlockInput {
            x: 0,
            y: 64,
            z: 0,
            item_slot: 8,
        };
        let result = handle_place_block(&state, &sender, input).await.unwrap();
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
        assert!(matches!(result, Err(BotError::Offline(_))));
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
        assert!(
            matches!(result, Err(BotError::InvalidParams(ref msg)) if msg.contains("out of bounds") || msg.contains("out of range"))
        );
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
        assert!(
            matches!(result, Err(BotError::InvalidParams(ref msg)) if msg.contains("must be 0-8"))
        );
    }

    #[tokio::test]
    async fn test_use_item_on_block_valid_no_slot() {
        let (state, sender) = make_echo_channel();
        make_online(&state);
        let input = UseItemOnBlockInput {
            x: 0,
            y: 64,
            z: 0,
            item_slot: None,
        };
        let result = handle_use_item_on_block(&state, &sender, input)
            .await
            .unwrap();
        let _: Value = serde_json::from_str(&result).expect("valid JSON");
    }

    #[tokio::test]
    async fn test_use_item_on_block_valid_with_slot() {
        // Verify the item_slot is propagated as BotCommand::UseItemOnBlock(pos, Some(3)).
        let state = Arc::new(SharedState::new(AppConfig::default()));
        make_online(&state);
        let (sender, mut receiver) = create_command_channel(4, Arc::clone(&state));

        let expected_pos = BlockPos::new(1, 64, 1);
        let responder = tokio::spawn(async move {
            let wrapped = receiver.recv().await.expect("should receive command");
            assert!(
                matches!(
                    wrapped.command,
                    BotCommand::UseItemOnBlock(pos, Some(3)) if pos == expected_pos
                ),
                "expected UseItemOnBlock({:?}, Some(3)), got: {:?}",
                expected_pos,
                wrapped.command
            );
            wrapped
                .respond_to
                .send(Ok(crate::types::BotResult {
                    success: true,
                    message: "ok".into(),
                    data: None,
                }))
                .expect("should respond");
        });

        let input = UseItemOnBlockInput {
            x: 1,
            y: 64,
            z: 1,
            item_slot: Some(3),
        };
        let result = handle_use_item_on_block(&state, &sender, input)
            .await
            .unwrap();
        let json: Value = serde_json::from_str(&result).expect("valid JSON");
        assert!(
            json.get("success")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            "expected success, got: {result}"
        );

        responder.await.expect("responder should finish");
    }

    // ── Creative-mode hint in descriptions ─────────────────────────

    #[test]
    fn test_creative_hint_in_descriptions() {
        assert!(
            BREAK_BLOCK_DESCRIPTION.contains(CREATIVE_MODE_HINT),
            "break_block description must contain the Creative-mode hint"
        );
        assert!(
            PLACE_BLOCK_DESCRIPTION.contains(CREATIVE_MODE_HINT),
            "place_block description must contain the Creative-mode hint"
        );
    }
}
