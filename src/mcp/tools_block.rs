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
pub const BREAK_BLOCK_DESCRIPTION: &str = "Break a block at the given position. By default (use_best_tool=true) runs the full compound mine flow (tool selection, movement, mining, verification) equivalent to act(Mine). In Creative mode, prefer `execute_command` with `/fill` or `/setblock` for bulk building.";

/// Full description for the `place_block` MCP tool.
///
/// The placed block occupies exactly `(x, y, z)`. `y` must be in
/// `-63..=320` — `y=-64` is rejected because the block is placed by
/// right-clicking the cell below (`y-1`), which would be outside the world.
pub const PLACE_BLOCK_DESCRIPTION: &str = "Place a block at the given position; the placed block occupies exactly (x, y, z), and y must be in -63..=320 (y=-64 is rejected — the clicked block would be at y=-65, outside the world). In Creative mode, prefer `execute_command` with `/fill` or `/setblock` for bulk building.";

// ── break_block ────────────────────────────────────────────────────────────

/// Input for the `break_block` MCP tool.
#[derive(Deserialize, Default, rmcp::schemars::JsonSchema)]
pub struct BreakBlockInput {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    /// Run the full compound mine flow (tool selection, movement, mining,
    /// verification) — equivalent to `act(Mine)`. Defaults to `true` so a
    /// bare `break_block` call behaves correctly (approaches the block,
    /// picks the right tool, and verifies the break). Set `false` for the
    /// raw fire-and-forget break.
    #[serde(default = "default_true")]
    pub use_best_tool: Option<bool>,
}

/// Serde default: `use_best_tool` defaults to `true` so direct `break_block`
/// calls get the reliable compound flow instead of the raw single packet.
fn default_true() -> Option<bool> {
    Some(true)
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
    let cmd = if input.use_best_tool.unwrap_or(true) {
        // perception_radius = Some(0): strip the nearby-blocks/entities
        // context so the response stays compact; the ActResult still carries
        // action_result, reason (e.g. ToolNotFound alternatives) and
        // self_info.position for drift awareness.
        BotCommand::Act(ActAction::Mine { block_pos: pos }, Some(0))
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
///
/// The placed block occupies exactly `(x, y, z)`. `y` must be in `-63..=320`:
/// `y=-64` is rejected because the block is placed by right-clicking the cell
/// below (`y-1`), which would be outside the world.
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
/// Validates coordinates (including the `place_block`-specific `y in -63..=320`
/// gate) and slot, checks online status, then sends
/// [`BotCommand::PlaceBlock`] with the target position — the position the
/// placed block occupies. The `item_slot` is encoded as `"slot:N"` in the
/// block type field so the bot executor can resolve the actual block type
/// from the player's inventory. The executor verifies the placement server
/// side; on `success:false` its failure message is returned verbatim (never
/// rewritten into a success-sounding sentence).
///
/// In Creative mode, prefer `execute_command` with `/fill` or `/setblock` for bulk building.
pub async fn handle_place_block(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    input: PlaceBlockInput,
) -> Result<String, BotError> {
    // place_block-specific Y gate: the block occupies exactly (x, y, z), so
    // the right-clicked block sits at y-1. y=-64 is impossible — the clicked
    // block would be at y=-65, outside the world — so the valid range is
    // -63..=320, one tighter than the generic -64..=320 block bounds.
    if !(-63..=320).contains(&input.y) {
        return Err(BotError::InvalidParams(format!(
            "y coordinate {} out of range for place_block (must be between -63 and 320; y=-64 is impossible — the clicked block would be at y=-65, outside the world)",
            input.y
        )));
    }

    // Validate coordinates are within world bounds (x/z border, y ceiling)
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
        Ok(mut result) => {
            // Rewrite the executor's message ONLY on success: the executor
            // now verifies the placement and returns honest `success:false`
            // results whose message names the failure (e.g. target cell
            // occupied, out of reach). Rewriting those would erase the
            // failure reason (M-7). On success, the executor only knows the
            // hotbar slot ("slot:N" encoding), so its message reads "Placed 3
            // at ..." — opaque to the LLM. Resolve the actual item id from
            // the snapshot inventory here (the MCP layer has snapshot access;
            // the executor does not) and rewrite the message to name it.
            if result.success {
                // F-33: the executor's success is already backed by the
                // server-confirmation poll, so the placement is authoritative
                // even when the throttled snapshot has not caught up yet.
                // Name the item when the snapshot inventory can resolve it;
                // otherwise fall back to the slot — never the misleading
                // "(empty slot)" label on a verified success.
                let item_id = state
                    .read_snapshot()
                    .self_player
                    .inventory
                    .iter()
                    .find(|entry| entry.slot_index == input.item_slot)
                    .map(|entry| entry.item_id.clone());
                result.message = match item_id {
                    Some(item_id) => format!(
                        "Placed {} at ({}, {}, {})",
                        item_id, input.x, input.y, input.z
                    ),
                    None => format!(
                        "Placed item from hotbar slot {} at ({}, {}, {}) (snapshot inventory not yet updated)",
                        input.item_slot, input.x, input.y, input.z
                    ),
                };
            }
            serde_json::to_string(&result)
                .map_err(|e| BotError::Internal(format!("Serialization error: {e}")))
        }
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
    /// Which face of the target block the item is used on; the placement
    /// lands in the cell that face opens into (default: "up", i.e. the cell
    /// above the target — e.g. pour a water bucket at (x, y+1, z) by using
    /// it on (x, y, z) with face up). One of: up, down, north, south, east,
    /// west. Only meaningful for placement items (buckets, blocks); other
    /// items ignore it.
    pub face: Option<String>,
}

/// Face direction for `use_item_on_block` — the cell the item's placement
/// lands in is `target + face_offset`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UseFace {
    Up,
    Down,
    North,
    South,
    East,
    West,
}

impl UseFace {
    /// Unit offset of this face: the cell the placement lands in.
    fn offset(self) -> (i32, i32, i32) {
        match self {
            UseFace::Up => (0, 1, 0),
            UseFace::Down => (0, -1, 0),
            UseFace::North => (0, 0, -1),
            UseFace::South => (0, 0, 1),
            UseFace::East => (1, 0, 0),
            UseFace::West => (-1, 0, 0),
        }
    }
}

/// Parse the `face` input string (case-insensitive). `None` → `Up`.
fn parse_use_face(value: Option<&str>) -> Result<UseFace, BotError> {
    match value.map(str::to_ascii_lowercase).as_deref() {
        None | Some("up") => Ok(UseFace::Up),
        Some("down") => Ok(UseFace::Down),
        Some("north") => Ok(UseFace::North),
        Some("south") => Ok(UseFace::South),
        Some("east") => Ok(UseFace::East),
        Some("west") => Ok(UseFace::West),
        Some(other) => Err(BotError::InvalidParams(format!(
            "face must be one of up/down/north/south/east/west, got {other:?}"
        ))),
    }
}

/// Handle `use_item_on_block` MCP tool.
///
/// Validates coordinates and optional slot, checks online status, then sends
/// [`BotCommand::UseItemOnBlock`]. If `item_slot` is provided, the bot
/// executor should switch to that slot before interacting.
///
/// The `face` argument (default `up`) declares which face of the target
/// block the item is used on. The expected placement cell is
/// `T = (x,y,z) + face_offset`; because the azalea interaction always
/// reports an Up face, the executor right-clicks the block below `T` and
/// verifies `T` turns non-air (server-side confirmation, no more fake
/// success for water buckets).
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

    // Parse the face and derive the interaction target + expected effect cell.
    let face = parse_use_face(input.face.as_deref())?;
    let (fx, fy, fz) = face.offset();
    let target = BlockPos::new(input.x, input.y, input.z);
    let effect = BlockPos::new(target.x + fx, target.y + fy, target.z + fz);
    if let Err(e) = validate_block_pos(&effect) {
        return Err(BotError::InvalidParams(e));
    }
    // azalea's `block_interact` reports a fixed Up face, so the placement
    // lands one cell above the right-clicked block: right-click the cell
    // BELOW the expected effect position.
    let interact_target = BlockPos::new(effect.x, effect.y - 1, effect.z);
    if let Err(e) = validate_block_pos(&interact_target) {
        return Err(BotError::InvalidParams(e));
    }

    if !state.is_online() {
        return Err(BotError::Offline(
            "Bot is not connected to a server".to_string(),
        ));
    }

    let cmd = BotCommand::UseItemOnBlock(interact_target, input.item_slot, Some(effect));
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

    /// Regression: the result message used to name only the hotbar slot
    /// ("Placed 3 at ..."), which is opaque to the LLM — it cannot know what
    /// block was placed. The MCP layer resolves the item id from the snapshot
    /// inventory and rewrites the message to "Placed <item_id> at ...".
    #[tokio::test]
    async fn test_place_block_message_names_item_from_inventory() {
        let (state, sender) = make_echo_channel();
        make_online(&state);
        // Give the snapshot a hotbar inventory so the handler can resolve
        // slot 3 → stone.
        state.update_snapshot(crate::types::WorldSnapshot {
            self_player: crate::types::SelfPlayer {
                inventory: vec![crate::types::InventorySlot {
                    slot_index: 3,
                    item_id: "stone".into(),
                    count: 64,
                }],
                ..Default::default()
            },
            ..Default::default()
        });
        let input = PlaceBlockInput {
            x: 5,
            y: 65,
            z: 10,
            item_slot: 3,
        };
        let result = handle_place_block(&state, &sender, input).await.unwrap();
        let v: Value = serde_json::from_str(&result).expect("valid JSON");
        assert_eq!(
            v["message"], "Placed stone at (5, 65, 10)",
            "message must name the block, not the slot: {v}"
        );
    }

    /// F-33: a verified-success result whose snapshot inventory has not yet
    /// caught up must not be relabelled "(empty slot)" — the executor's
    /// server-confirmation poll is the authority. It falls back to the slot
    /// with an explicit "snapshot not yet updated" qualifier.
    #[tokio::test]
    async fn test_place_block_message_missing_snapshot_inventory_keeps_slot() {
        let (state, sender) = make_echo_channel();
        make_online(&state);
        // Snapshot inventory does not contain slot 7.
        let input = PlaceBlockInput {
            x: 0,
            y: 64,
            z: 0,
            item_slot: 7,
        };
        let result = handle_place_block(&state, &sender, input).await.unwrap();
        let v: Value = serde_json::from_str(&result).expect("valid JSON");
        let message = v["message"].as_str().expect("message present");
        assert!(
            message.contains("hotbar slot 7") && !message.contains("(empty slot)"),
            "a verified success must never be labelled empty slot: {message}"
        );
    }

    /// RED (H-2 MCP side): `place_block` validates `y in -63..=320` — the
    /// block occupies exactly (x, y, z), so the right-clicked block is at
    /// y-1. `y=-64` would put the clicked block at y=-65, outside the world,
    /// and must be rejected with a message explaining the limitation, while
    /// `y=-63` (the lowest placeable cell) is accepted.
    #[tokio::test]
    async fn test_place_block_y_minus_64_rejected() {
        let (state, sender) = setup();
        make_online(&state);
        let input = PlaceBlockInput {
            x: 0,
            y: -64,
            z: 0,
            item_slot: 0,
        };
        let result = handle_place_block(&state, &sender, input).await;
        match result {
            Err(BotError::InvalidParams(msg)) => {
                assert!(msg.contains("-64"), "msg should mention y=-64, got: {msg}");
                assert!(
                    msg.contains("world"),
                    "msg should explain the world-boundary reason, got: {msg}"
                );
            }
            other => panic!("expected InvalidParams for y=-64, got {other:?}"),
        }

        // y=-63 is the lowest placeable cell and must dispatch normally.
        let (state2, sender2) = make_echo_channel();
        make_online(&state2);
        let input = PlaceBlockInput {
            x: 0,
            y: -63,
            z: 0,
            item_slot: 0,
        };
        let result = handle_place_block(&state2, &sender2, input)
            .await
            .expect("y=-63 must be accepted");
        let _: Value = serde_json::from_str(&result).expect("valid JSON");
    }

    /// RED (M-7): the executor verifies placement and can return
    /// `success:false` with a message naming the failure (e.g. target cell
    /// occupied, out of reach). The MCP layer must NOT rewrite that message
    /// into a success-sounding "Placed ... at ..." sentence — the failure
    /// reason must survive verbatim.
    #[tokio::test]
    async fn test_place_block_failure_message_not_rewritten() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        make_online(&state);
        // Give the snapshot a hotbar inventory so a rewrite WOULD have been
        // possible — proving the rewrite is skipped, not just unresolved.
        state.update_snapshot(crate::types::WorldSnapshot {
            self_player: crate::types::SelfPlayer {
                inventory: vec![crate::types::InventorySlot {
                    slot_index: 3,
                    item_id: "stone".into(),
                    count: 64,
                }],
                ..Default::default()
            },
            ..Default::default()
        });
        let (sender, mut receiver) = create_command_channel(4, Arc::clone(&state));

        let responder = tokio::spawn(async move {
            let wrapped = receiver.recv().await.expect("should receive command");
            wrapped
                .respond_to
                .send(Ok(crate::types::BotResult {
                    success: false,
                    message: "cannot place: target cell occupied by stone".into(),
                    data: None,
                }))
                .expect("should respond");
        });

        let input = PlaceBlockInput {
            x: 5,
            y: 65,
            z: 10,
            item_slot: 3,
        };
        let result = handle_place_block(&state, &sender, input)
            .await
            .expect("handler should return the executor's result");
        let v: Value = serde_json::from_str(&result).expect("valid JSON");
        assert_eq!(v["success"], false, "failure must stay a failure: {v}");
        assert_eq!(
            v["message"], "cannot place: target cell occupied by stone",
            "failure message must survive verbatim, got: {v}"
        );
        responder.await.expect("responder finished");
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
            face: None,
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
            face: None,
        };
        let result = handle_use_item_on_block(&state, &sender, input).await;
        assert!(
            matches!(result, Err(BotError::InvalidParams(ref msg)) if msg.contains("out of bounds") || msg.contains("out of range"))
        );
    }

    #[tokio::test]
    async fn test_use_item_on_block_face_up_at_build_limit_rejected() {
        // face=up at y=320 would place the effect cell at y=321, outside the
        // build height. This must be rejected before dispatch.
        let (state, sender) = setup();
        make_online(&state);
        let input = UseItemOnBlockInput {
            x: 0,
            y: 320,
            z: 0,
            item_slot: None,
            face: Some("up".into()),
        };
        let result = handle_use_item_on_block(&state, &sender, input).await;
        assert!(
            matches!(result, Err(BotError::InvalidParams(ref msg)) if msg.contains("out of bounds") || msg.contains("out of range")),
            "got: {result:?}"
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
            face: None,
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
            face: None,
        };
        let result = handle_use_item_on_block(&state, &sender, input)
            .await
            .unwrap();
        let _: Value = serde_json::from_str(&result).expect("valid JSON");
    }

    #[tokio::test]
    async fn test_use_item_on_block_valid_with_slot() {
        // Verify item_slot + face(default up) are propagated: the command's
        // interaction target is the block below the expected effect cell.
        let state = Arc::new(SharedState::new(AppConfig::default()));
        make_online(&state);
        let (sender, mut receiver) = create_command_channel(4, Arc::clone(&state));

        let expected_pos = BlockPos::new(1, 64, 1);
        let expected_effect = BlockPos::new(1, 65, 1);
        let responder = tokio::spawn(async move {
            let wrapped = receiver.recv().await.expect("should receive command");
            assert!(
                matches!(
                    wrapped.command,
                    BotCommand::UseItemOnBlock(pos, Some(3), Some(effect))
                        if pos == expected_pos && effect == expected_effect
                ),
                "expected UseItemOnBlock({:?}, Some(3), Some({:?})), got: {:?}",
                expected_pos,
                expected_effect,
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
            face: None,
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

    // ── use_item_on_block: face handling ─────────────────────────

    #[test]
    fn test_parse_use_face_defaults_to_up() {
        assert_eq!(parse_use_face(None).unwrap(), UseFace::Up);
        assert_eq!(parse_use_face(Some("up")).unwrap(), UseFace::Up);
        assert_eq!(parse_use_face(Some("UP")).unwrap(), UseFace::Up);
    }

    #[test]
    fn test_parse_use_face_all_directions() {
        assert_eq!(parse_use_face(Some("down")).unwrap(), UseFace::Down);
        assert_eq!(parse_use_face(Some("north")).unwrap(), UseFace::North);
        assert_eq!(parse_use_face(Some("south")).unwrap(), UseFace::South);
        assert_eq!(parse_use_face(Some("east")).unwrap(), UseFace::East);
        assert_eq!(parse_use_face(Some("west")).unwrap(), UseFace::West);
    }

    #[test]
    fn test_parse_use_face_rejects_unknown() {
        let err = parse_use_face(Some("sideways")).unwrap_err();
        assert!(matches!(err, BotError::InvalidParams(_)));
    }

    #[test]
    fn test_use_face_offsets() {
        assert_eq!(UseFace::Up.offset(), (0, 1, 0));
        assert_eq!(UseFace::Down.offset(), (0, -1, 0));
        assert_eq!(UseFace::North.offset(), (0, 0, -1));
        assert_eq!(UseFace::South.offset(), (0, 0, 1));
        assert_eq!(UseFace::East.offset(), (1, 0, 0));
        assert_eq!(UseFace::West.offset(), (-1, 0, 0));
    }

    #[tokio::test]
    async fn test_use_item_on_block_face_down_targets_cell_below() {
        // face=down: effect lands at (x, y-1, z); the interaction target is
        // the cell below that — (x, y-2, z) — because azalea always reports
        // an Up face.
        let state = Arc::new(SharedState::new(AppConfig::default()));
        make_online(&state);
        let (sender, mut receiver) = create_command_channel(4, Arc::clone(&state));
        let responder = tokio::spawn(async move {
            let wrapped = receiver.recv().await.expect("should receive command");
            assert_eq!(
                wrapped.command,
                BotCommand::UseItemOnBlock(
                    BlockPos::new(3, 62, 5),
                    None,
                    Some(BlockPos::new(3, 63, 5)),
                )
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
            x: 3,
            y: 64,
            z: 5,
            item_slot: None,
            face: Some("down".into()),
        };
        handle_use_item_on_block(&state, &sender, input)
            .await
            .expect("handler should succeed");
        // The responder task asserts the dispatched command; wait for it so
        // assertion failures surface in the test.
        responder.await.expect("responder task should finish");
    }

    #[tokio::test]
    async fn test_use_item_on_block_invalid_face_rejected() {
        let (state, sender) = setup();
        make_online(&state);
        let input = UseItemOnBlockInput {
            x: 0,
            y: 64,
            z: 0,
            item_slot: None,
            face: Some("sideways".into()),
        };
        let result = handle_use_item_on_block(&state, &sender, input).await;
        assert!(
            matches!(result, Err(BotError::InvalidParams(ref msg)) if msg.contains("face")),
            "got: {result:?}"
        );
    }
}
