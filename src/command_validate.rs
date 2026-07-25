//! Input validation for commands and tool parameters.
//!
//! This module provides validation functions that check command parameters
//! for correctness before they are passed to the bot engine. Validation is
//! stateless �?it only checks that the values themselves are within expected
//! ranges, without consulting any external state.

use crate::error::BotError;
use crate::types::{ActAction, BlockPos, BotCommand};

// ── World bounds constants ────────────────────────────────────────────────

/// Minecraft world border half-extent on X and Z axes.
const WORLD_BORDER: i32 = 30_000_000;

/// Minimum build height (Y level).
const MIN_Y: i32 = -64;

/// Maximum build height (Y level).
const MAX_Y: i32 = 320;

// ── Public validation API ─────────────────────────────────────────────────

/// Saturating cast from `u32` to `i32`, capping at `i32::MAX`.
///
/// Use this anywhere an external input is held as `u32` but later compared
/// as a signed offset (e.g. Chebyshev radius filtering). A bare `v as i32`
/// silently wraps values above `i32::MAX` into negative numbers, which makes
/// the comparison filter return nothing — see P0-#1 in
/// `.trae/specs/fix-audit-issues/spec.md`.
///
/// Inputs already in the safe range pass through unchanged. Inputs above
/// `i32::MAX` are clamped to `i32::MAX`; behaviour does not panic.
pub fn clamp_to_i32(v: u32) -> i32 {
    if v > i32::MAX as u32 {
        i32::MAX
    } else {
        v as i32
    }
}

/// Validate a [`BotCommand`] before execution.
///
/// Returns `Ok(())` if all parameters are within acceptable ranges, or
/// `Err(BotError)` describing the first validation failure.
///
/// # Errors
///
/// Returns [`BotError::InvalidParams`] when any parameter is out of bounds
/// or otherwise invalid. This maps to MCP `INVALID_PARAMS` so clients can
/// distinguish user input errors from internal failures.
pub fn validate_command(cmd: &BotCommand) -> Result<(), BotError> {
    match cmd {
        // Position-based commands: all validate position bounds
        BotCommand::MoveTo(pos)
        | BotCommand::Teleport(pos)
        | BotCommand::BreakBlock(pos)
        | BotCommand::PlaceBlock(pos, _)
        | BotCommand::OpenContainer(pos) => validate_position(pos),

        // UseItemOnBlock additionally takes an optional hotbar slot (0-8).
        BotCommand::UseItemOnBlock(pos, slot) => {
            validate_position(pos)?;
            if let Some(s) = slot
                && *s > 8
            {
                return Err(BotError::InvalidParams(format!(
                    "UseItemOnBlock hotbar slot must be 0-8, got {s}"
                )));
            }
            Ok(())
        }

        // Direction is verified by the type system; distance is bounded to
        // keep the pathfinder target within a sane range (1..=1000 blocks).
        BotCommand::WalkDirection(_, distance) => {
            if *distance == 0 || *distance > 1000 {
                return Err(BotError::InvalidParams(format!(
                    "distance must be 1..=1000, got {distance}"
                )));
            }
            Ok(())
        }

        // AttackEntity: entity_id must fit in i32::MAX. azalea's entity
        // lookups cast u32 → i32; values > i32::MAX would wrap to negative
        // IDs and silently never match a real entity.
        BotCommand::AttackEntity(entity_id) => {
            if *entity_id > i32::MAX as u32 {
                return Err(BotError::InvalidParams(format!(
                    "entity_id must be <= i32::MAX ({}), got {entity_id}",
                    i32::MAX
                )));
            }
            Ok(())
        }

        // Parameterless commands — always valid.
        BotCommand::Jump
        | BotCommand::UseItem
        | BotCommand::EquipTool(_)
        | BotCommand::EquipToolWithMaterial(_, _)
        | BotCommand::CloseContainer
        | BotCommand::ShieldBlock(_)
        | BotCommand::SetGameMode(_)
        | BotCommand::QuerySelfInfo
        | BotCommand::QueryInventory
        | BotCommand::QueryChunkSummary => Ok(()),

        // Hotbar slot must be in range 0-8.
        BotCommand::SwitchHotbarSlot(slot) => {
            if *slot > 8 {
                return Err(BotError::InvalidParams(format!(
                    "Hotbar slot must be 0-8, got {slot}"
                )));
            }
            Ok(())
        }

        // Atomic switch + use item: same slot bounds as SwitchHotbarSlot.
        BotCommand::UseItemWithSlot(slot) => {
            if *slot > 8 {
                return Err(BotError::InvalidParams(format!(
                    "item_slot must be 0..=8, got {slot}"
                )));
            }
            Ok(())
        }

        // Slotted operations require a valid slot and a positive count (1-64).
        // DropItem targets the player inventory (hotbar 0-8, main 9-35).
        BotCommand::DropItem(slot, count) => {
            if *slot > 35 {
                return Err(BotError::InvalidParams(format!(
                    "DropItem slot must be 0-35, got {slot}"
                )));
            }
            if *count == 0 || *count > 64 {
                return Err(BotError::InvalidParams(format!(
                    "DropItem count must be 1-64, got {count}"
                )));
            }
            Ok(())
        }

        // Container slots can go up to 53 (double chests have 54 slots, 0-53).
        BotCommand::TakeFromContainer(slot, count) => {
            if *slot > 53 {
                return Err(BotError::InvalidParams(format!(
                    "TakeFromContainer slot must be 0-53, got {slot}"
                )));
            }
            if *count == 0 || *count > 64 {
                return Err(BotError::InvalidParams(format!(
                    "TakeFromContainer count must be 1-64, got {count}"
                )));
            }
            Ok(())
        }

        // PutIntoContainer mirrors TakeFromContainer's bounds.
        BotCommand::PutIntoContainer(slot, count) => {
            if *slot > 53 {
                return Err(BotError::InvalidParams(format!(
                    "PutIntoContainer slot must be 0-53, got {slot}"
                )));
            }
            if *count == 0 || *count > 64 {
                return Err(BotError::InvalidParams(format!(
                    "PutIntoContainer count must be 1-64, got {count}"
                )));
            }
            Ok(())
        }

        // Messages must be non-empty (whitespace-only also rejected).
        BotCommand::SendChat(msg) | BotCommand::ExecuteCommand(msg) => {
            if msg.trim().is_empty() {
                return Err(BotError::InvalidParams("Message cannot be empty".into()));
            }
            Ok(())
        }

        // Nearby block and entity queries use a capped radius consistent
        // with the MCP layer (R-4: 1..=100) to prevent pathological O(n) scans.
        BotCommand::QueryNearbyBlocks(radius) => {
            if *radius < 1 || *radius > 100 {
                return Err(BotError::InvalidParams(format!(
                    "Radius must be between 1 and 100, got {radius}"
                )));
            }
            Ok(())
        }

        BotCommand::QueryNearbyEntities(radius) => {
            if *radius < 1 || *radius > 100 {
                return Err(BotError::InvalidParams(format!(
                    "Radius must be between 1 and 100, got {radius}"
                )));
            }
            Ok(())
        }

        // ── v2 foundation variants ──────────────────────────────────────

        // Smart movement and flight use the same position bounds as MoveTo.
        BotCommand::SmartMove(pos) | BotCommand::FlyTo(pos) => validate_position(pos),

        // Item pickup radius must be positive and capped at 64 blocks.
        BotCommand::CollectItems(radius) => {
            if *radius == 0 || *radius > 64 {
                return Err(BotError::InvalidParams(format!(
                    "CollectItems radius must be between 1 and 64, got {radius}"
                )));
            }
            Ok(())
        }

        // Parameterless queries — always valid.
        BotCommand::QueryServerInfo | BotCommand::QueryChatHistory => Ok(()),

        // World view radius is capped at 32 chunks.
        BotCommand::QueryWorldView(radius) => {
            if *radius == 0 || *radius > 32 {
                return Err(BotError::InvalidParams(format!(
                    "QueryWorldView radius must be between 1 and 32, got {radius}"
                )));
            }
            Ok(())
        }

        // Unified Act tool — delegate to the inner action's validation.
        BotCommand::Act(action) => validate_act_action(action),
    }
}

/// Validate an [`ActAction`] payload by delegating to the equivalent
/// standalone command's validation.
///
/// Each variant is checked against the same bounds the underlying
/// [`BotCommand`] would use:
///
/// - Position-bearing variants ([`ActAction::Move`], [`ActAction::SmartMove`],
///   [`ActAction::Fly`], [`ActAction::Mine`]) must respect the Minecraft
///   world border (X / Z) and build-height limits (Y).
/// - [`ActAction::Attack::entity_id`] must fit in `i32::MAX`; azalea's entity
///   lookups cast `u32` → `i32` and values above `i32::MAX` would wrap to
///   negative IDs that never match a real entity.
/// - [`ActAction::CollectItems::radius`] is bounded 1..=64 to match
///   [`BotCommand::CollectItems`] and to keep any `as i32` filter inside
///   that handler safe via [`clamp_to_i32`].
///
/// This function is `pub` so MCP handlers (e.g. `tools_act::handle_act`) can
/// validate `ActAction` payloads before dispatching them.
pub fn validate_act_action(action: &ActAction) -> Result<(), BotError> {
    match action {
        ActAction::Move { target }
        | ActAction::SmartMove { target }
        | ActAction::Fly { target } => validate_position(target),
        ActAction::Mine { block_pos } => validate_position(block_pos),
        ActAction::Attack { entity_id } => {
            if *entity_id > i32::MAX as u32 {
                return Err(BotError::InvalidParams(format!(
                    "Attack entity_id must be <= i32::MAX ({}), got {entity_id}",
                    i32::MAX
                )));
            }
            Ok(())
        }
        ActAction::CollectItems { radius } => {
            if *radius == 0 || *radius > 64 {
                return Err(BotError::InvalidParams(format!(
                    "CollectItems radius must be between 1 and 64, got {radius}"
                )));
            }
            Ok(())
        }
    }
}

/// Validates that the given coordinates are within Minecraft world bounds.
///
/// Returns `Ok(())` if valid, or `Err(String)` with a descriptive message.
///
/// World bounds:
/// - X / Z: ±30,000,000 (world border)
/// - Y: -64 to +320 (build height limits)
///
/// Note: comparisons use explicit lower/upper bounds rather than `x.abs()`
/// to avoid the `i32::MIN` overflow (`.abs()` panics in debug and wraps in
/// release, letting `i32::MIN` slip past an `abs()`-based check).
pub fn validate_coordinates(x: i32, y: i32, z: i32) -> Result<(), String> {
    if !(-WORLD_BORDER..=WORLD_BORDER).contains(&x) {
        return Err(format!(
            "x coordinate {x} out of range (must be between -{WORLD_BORDER} and {WORLD_BORDER})"
        ));
    }
    if !(-WORLD_BORDER..=WORLD_BORDER).contains(&z) {
        return Err(format!(
            "z coordinate {z} out of range (must be between -{WORLD_BORDER} and {WORLD_BORDER})"
        ));
    }
    if !(MIN_Y..=MAX_Y).contains(&y) {
        return Err(format!(
            "y coordinate {y} out of range (must be between {MIN_Y} and {MAX_Y})"
        ));
    }
    Ok(())
}

/// Validates a [`BlockPos`] is within Minecraft world bounds.
///
/// Returns `Ok(())` if valid, or `Err(String)` with a descriptive message.
pub fn validate_block_pos(pos: &BlockPos) -> Result<(), String> {
    validate_coordinates(pos.x, pos.y, pos.z)
}

// ── Internal helpers ──────────────────────────────────────────────────────

/// Validate that a [`BlockPos`] is within the Minecraft world bounds.
///
/// World bounds:
/// - X / Z: ±30,000,000 (world border)
/// - Y: -64 to +320 (build height limits)
fn validate_position(pos: &BlockPos) -> Result<(), BotError> {
    if pos.x < -WORLD_BORDER || pos.x > WORLD_BORDER {
        return Err(BotError::InvalidParams(format!(
            "X coordinate {} out of bounds (must be between {} and {})",
            pos.x, -WORLD_BORDER, WORLD_BORDER
        )));
    }
    if pos.y < MIN_Y || pos.y > MAX_Y {
        return Err(BotError::InvalidParams(format!(
            "Y coordinate {} out of bounds (must be between {MIN_Y} and {MAX_Y})",
            pos.y,
        )));
    }
    if pos.z < -WORLD_BORDER || pos.z > WORLD_BORDER {
        return Err(BotError::InvalidParams(format!(
            "Z coordinate {} out of bounds (must be between {} and {})",
            pos.z, -WORLD_BORDER, WORLD_BORDER
        )));
    }
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ActAction, Direction, GameMode, MaterialTier, ToolType};

    // ── Position validation ─────────────────────────────────────────

    #[test]
    fn test_validate_position_origin_is_valid() {
        let pos = BlockPos::new(0, 0, 0);
        assert!(validate_position(&pos).is_ok());
    }

    #[test]
    fn test_validate_position_valid_coordinates() {
        let pos = BlockPos::new(100, 64, -200);
        assert!(validate_position(&pos).is_ok());
    }

    #[test]
    fn test_validate_position_x_too_low() {
        let pos = BlockPos::new(-30_000_001, 0, 0);
        assert!(validate_position(&pos).is_err());
    }

    #[test]
    fn test_validate_position_x_too_high() {
        let pos = BlockPos::new(30_000_001, 0, 0);
        assert!(validate_position(&pos).is_err());
    }

    #[test]
    fn test_validate_position_y_too_low() {
        let pos = BlockPos::new(0, -65, 0);
        assert!(validate_position(&pos).is_err());
    }

    #[test]
    fn test_validate_position_y_too_high() {
        let pos = BlockPos::new(0, 321, 0);
        assert!(validate_position(&pos).is_err());
    }

    #[test]
    fn test_validate_position_y_min_boundary() {
        let pos = BlockPos::new(0, -64, 0);
        assert!(validate_position(&pos).is_ok());
    }

    #[test]
    fn test_validate_position_y_max_boundary() {
        let pos = BlockPos::new(0, 320, 0);
        assert!(validate_position(&pos).is_ok());
    }

    #[test]
    fn test_validate_position_z_too_low() {
        let pos = BlockPos::new(0, 0, -30_000_001);
        assert!(validate_position(&pos).is_err());
    }

    #[test]
    fn test_validate_position_z_too_high() {
        let pos = BlockPos::new(0, 0, 30_000_001);
        assert!(validate_position(&pos).is_err());
    }

    #[test]
    fn test_validate_position_edge_x_z() {
        let pos = BlockPos::new(30_000_000, 64, -30_000_000);
        assert!(validate_position(&pos).is_ok());
    }

    // ── validate_coordinates ────────────────────────────────────────

    #[test]
    fn test_validate_coordinates_valid() {
        assert!(validate_coordinates(0, 64, 0).is_ok());
        assert!(validate_coordinates(100, 100, 100).is_ok());
        assert!(validate_coordinates(-100, -50, -100).is_ok());
        assert!(validate_coordinates(WORLD_BORDER, MAX_Y, WORLD_BORDER).is_ok());
        assert!(validate_coordinates(-WORLD_BORDER, MIN_Y, -WORLD_BORDER).is_ok());
    }

    #[test]
    fn test_validate_coordinates_x_out_of_range() {
        assert!(validate_coordinates(WORLD_BORDER + 1, 64, 0).is_err());
        assert!(validate_coordinates(-WORLD_BORDER - 1, 64, 0).is_err());
    }

    #[test]
    fn test_validate_coordinates_z_out_of_range() {
        assert!(validate_coordinates(0, 64, WORLD_BORDER + 1).is_err());
        assert!(validate_coordinates(0, 64, -WORLD_BORDER - 1).is_err());
    }

    #[test]
    fn test_validate_coordinates_y_out_of_range() {
        assert!(validate_coordinates(0, MIN_Y - 1, 0).is_err());
        assert!(validate_coordinates(0, MAX_Y + 1, 0).is_err());
    }

    #[test]
    fn test_validate_coordinates_i32_min_no_overflow() {
        // Regression: `i32::MIN.abs()` overflows (panics in debug, wraps in
        // release). The explicit-bound comparison must reject it cleanly.
        assert!(validate_coordinates(i32::MIN, 64, 0).is_err());
        assert!(validate_coordinates(0, 64, i32::MIN).is_err());
        // i32::MAX is also out of bounds (> 30_000_000) and must be rejected.
        assert!(validate_coordinates(i32::MAX, 64, 0).is_err());
        assert!(validate_coordinates(0, 64, i32::MAX).is_err());
    }

    #[test]
    fn test_validate_block_pos() {
        let valid = BlockPos::new(10, 64, 10);
        assert!(validate_block_pos(&valid).is_ok());

        let invalid = BlockPos::new(WORLD_BORDER + 1, 64, 0);
        assert!(validate_block_pos(&invalid).is_err());
    }

    // ── Position-based commands ────────────────────────────────────

    #[test]
    fn test_move_to_valid() {
        let cmd = BotCommand::MoveTo(BlockPos::new(10, 64, 20));
        assert!(validate_command(&cmd).is_ok());
    }

    #[test]
    fn test_move_to_invalid_y() {
        let cmd = BotCommand::MoveTo(BlockPos::new(0, 500, 0));
        assert!(validate_command(&cmd).is_err());
    }

    #[test]
    fn test_teleport_valid() {
        let cmd = BotCommand::Teleport(BlockPos::new(0, 64, 0));
        assert!(validate_command(&cmd).is_ok());
    }

    #[test]
    fn test_teleport_invalid_x() {
        let cmd = BotCommand::Teleport(BlockPos::new(99_999_999, 0, 0));
        assert!(validate_command(&cmd).is_err());
    }

    #[test]
    fn test_break_block_valid() {
        let cmd = BotCommand::BreakBlock(BlockPos::new(0, -64, 0));
        assert!(validate_command(&cmd).is_ok());
    }

    #[test]
    fn test_break_block_invalid() {
        let cmd = BotCommand::BreakBlock(BlockPos::new(0, -65, 0));
        assert!(validate_command(&cmd).is_err());
    }

    #[test]
    fn test_place_block_valid() {
        let cmd = BotCommand::PlaceBlock(BlockPos::new(0, 320, 0), "stone".into());
        assert!(validate_command(&cmd).is_ok());
    }

    #[test]
    fn test_place_block_invalid() {
        let cmd = BotCommand::PlaceBlock(BlockPos::new(0, 321, 0), "stone".into());
        assert!(validate_command(&cmd).is_err());
    }

    #[test]
    fn test_use_item_on_block_valid() {
        let cmd = BotCommand::UseItemOnBlock(BlockPos::new(0, 0, 0), None);
        assert!(validate_command(&cmd).is_ok());
    }

    #[test]
    fn test_use_item_on_block_invalid_x() {
        let cmd = BotCommand::UseItemOnBlock(BlockPos::new(30_000_001, 0, 0), None);
        assert!(validate_command(&cmd).is_err());
    }

    #[test]
    fn test_use_item_on_block_valid_slot() {
        // Hotbar slot 0-8 is valid.
        for slot in 0..=8u8 {
            let cmd = BotCommand::UseItemOnBlock(BlockPos::new(0, 0, 0), Some(slot));
            assert!(
                validate_command(&cmd).is_ok(),
                "UseItemOnBlock slot {slot} should be valid"
            );
        }
    }

    #[test]
    fn test_use_item_on_block_invalid_slot() {
        // Slot 9 is outside the hotbar range.
        let cmd = BotCommand::UseItemOnBlock(BlockPos::new(0, 0, 0), Some(9));
        assert!(validate_command(&cmd).is_err());
        // u8::MAX is far out of range.
        let cmd = BotCommand::UseItemOnBlock(BlockPos::new(0, 0, 0), Some(u8::MAX));
        assert!(validate_command(&cmd).is_err());
    }

    #[test]
    fn test_open_container_valid() {
        let cmd = BotCommand::OpenContainer(BlockPos::new(10, 64, -10));
        assert!(validate_command(&cmd).is_ok());
    }

    #[test]
    fn test_open_container_invalid() {
        let cmd = BotCommand::OpenContainer(BlockPos::new(0, -65, 0));
        assert!(validate_command(&cmd).is_err());
    }

    // ── Hotbar slot ───────────────────────────────────────────────

    #[test]
    fn test_switch_hotbar_slot_valid_range() {
        for slot in 0..=8u8 {
            let cmd = BotCommand::SwitchHotbarSlot(slot);
            assert!(
                validate_command(&cmd).is_ok(),
                "hotbar slot {slot} should be valid"
            );
        }
    }

    #[test]
    fn test_switch_hotbar_slot_too_high() {
        let cmd = BotCommand::SwitchHotbarSlot(9);
        assert!(validate_command(&cmd).is_err());
    }

    #[test]
    fn test_switch_hotbar_slot_max_u8() {
        let cmd = BotCommand::SwitchHotbarSlot(u8::MAX);
        assert!(validate_command(&cmd).is_err());
    }

    // ── Message validation ────────────────────────────────────────

    #[test]
    fn test_send_chat_valid() {
        let cmd = BotCommand::SendChat("hello".into());
        assert!(validate_command(&cmd).is_ok());
    }

    #[test]
    fn test_send_chat_empty_string() {
        let cmd = BotCommand::SendChat(String::new());
        assert!(validate_command(&cmd).is_err());
    }

    #[test]
    fn test_send_chat_whitespace_only() {
        let cmd = BotCommand::SendChat("   ".into());
        assert!(validate_command(&cmd).is_err());
    }

    #[test]
    fn test_execute_command_valid() {
        let cmd = BotCommand::ExecuteCommand("/gamemode creative".into());
        assert!(validate_command(&cmd).is_ok());
    }

    #[test]
    fn test_execute_command_empty() {
        let cmd = BotCommand::ExecuteCommand(String::new());
        assert!(validate_command(&cmd).is_err());
    }

    // ── Radius validation ─────────────────────────────────────────

    #[test]
    fn test_query_nearby_blocks_valid_range() {
        for radius in 1..=100u32 {
            let cmd = BotCommand::QueryNearbyBlocks(radius);
            assert!(
                validate_command(&cmd).is_ok(),
                "block query radius {radius} should be valid"
            );
        }
    }

    #[test]
    fn test_query_nearby_blocks_zero() {
        let cmd = BotCommand::QueryNearbyBlocks(0);
        assert!(validate_command(&cmd).is_err());
    }

    #[test]
    fn test_query_nearby_blocks_too_large() {
        let cmd = BotCommand::QueryNearbyBlocks(101);
        assert!(validate_command(&cmd).is_err());
    }

    #[test]
    fn test_query_nearby_entities_valid() {
        for radius in 1..=100u32 {
            let cmd = BotCommand::QueryNearbyEntities(radius);
            assert!(
                validate_command(&cmd).is_ok(),
                "entity query radius {radius} should be valid"
            );
        }
    }

    #[test]
    fn test_query_nearby_entities_zero() {
        let cmd = BotCommand::QueryNearbyEntities(0);
        assert!(validate_command(&cmd).is_err());
    }

    #[test]
    fn test_query_nearby_entities_large() {
        // Entity queries allow up to 100 (R-4, consistent with MCP layer).
        let cmd = BotCommand::QueryNearbyEntities(100);
        assert!(validate_command(&cmd).is_ok());
    }

    #[test]
    fn test_query_nearby_entities_too_large() {
        // Values > 100 are rejected to prevent pathological O(n) scans.
        let cmd = BotCommand::QueryNearbyEntities(101);
        assert!(validate_command(&cmd).is_err());
        // u32::MAX must also be rejected.
        let cmd = BotCommand::QueryNearbyEntities(u32::MAX);
        assert!(validate_command(&cmd).is_err());
    }

    // ── Pass-through commands (always valid) ───────────────────────

    #[test]
    fn test_jump_valid() {
        assert!(validate_command(&BotCommand::Jump).is_ok());
    }

    #[test]
    fn test_walk_direction_valid() {
        let cmd = BotCommand::WalkDirection(Direction::North, 1);
        assert!(validate_command(&cmd).is_ok());
    }

    #[test]
    fn test_use_item_valid() {
        assert!(validate_command(&BotCommand::UseItem).is_ok());
    }

    #[test]
    fn test_equip_tool_valid() {
        let cmd = BotCommand::EquipTool(ToolType::Pickaxe);
        assert!(validate_command(&cmd).is_ok());
    }

    #[test]
    fn test_equip_tool_with_material_valid() {
        let cmd = BotCommand::EquipToolWithMaterial(ToolType::Pickaxe, MaterialTier::Diamond);
        assert!(validate_command(&cmd).is_ok());
    }

    #[test]
    fn test_close_container_valid() {
        assert!(validate_command(&BotCommand::CloseContainer).is_ok());
    }

    #[test]
    fn test_attack_entity_valid() {
        let cmd = BotCommand::AttackEntity(42);
        assert!(validate_command(&cmd).is_ok());
    }

    #[test]
    fn test_shield_block_valid() {
        assert!(validate_command(&BotCommand::ShieldBlock(true)).is_ok());
    }

    #[test]
    fn test_set_game_mode_valid() {
        let cmd = BotCommand::SetGameMode(GameMode::Creative);
        assert!(validate_command(&cmd).is_ok());
    }

    #[test]
    fn test_query_self_info_valid() {
        assert!(validate_command(&BotCommand::QuerySelfInfo).is_ok());
    }

    #[test]
    fn test_query_inventory_valid() {
        assert!(validate_command(&BotCommand::QueryInventory).is_ok());
    }

    #[test]
    fn test_query_chunk_summary_valid() {
        assert!(validate_command(&BotCommand::QueryChunkSummary).is_ok());
    }

    // ── DropItem / Container slot operations ───────────────────────

    #[test]
    fn test_drop_item_valid() {
        let cmd = BotCommand::DropItem(1, 1);
        assert!(validate_command(&cmd).is_ok());
    }

    #[test]
    fn test_drop_item_zero_count() {
        let cmd = BotCommand::DropItem(1, 0);
        assert!(validate_command(&cmd).is_err());
    }

    #[test]
    fn test_drop_item_count_too_large() {
        let cmd = BotCommand::DropItem(1, 65);
        assert!(validate_command(&cmd).is_err());
        let cmd = BotCommand::DropItem(1, u8::MAX);
        assert!(validate_command(&cmd).is_err());
    }

    #[test]
    fn test_drop_item_valid_count_boundary() {
        // 64 is the Minecraft stack limit — must be accepted.
        let cmd = BotCommand::DropItem(1, 64);
        assert!(validate_command(&cmd).is_ok());
    }

    #[test]
    fn test_drop_item_valid_slot_boundary() {
        // Player inventory spans hotbar 0-8 and main 9-35.
        let cmd = BotCommand::DropItem(35, 1);
        assert!(validate_command(&cmd).is_ok());
    }

    #[test]
    fn test_drop_item_slot_too_high() {
        // Slot 36 is outside the player inventory range.
        let cmd = BotCommand::DropItem(36, 1);
        assert!(validate_command(&cmd).is_err());
        // u8::MAX is far out of range.
        let cmd = BotCommand::DropItem(u8::MAX, 1);
        assert!(validate_command(&cmd).is_err());
    }

    #[test]
    fn test_take_from_container_valid() {
        let cmd = BotCommand::TakeFromContainer(0, 1);
        assert!(validate_command(&cmd).is_ok());
    }

    #[test]
    fn test_take_from_container_zero_count() {
        let cmd = BotCommand::TakeFromContainer(0, 0);
        assert!(validate_command(&cmd).is_err());
    }

    #[test]
    fn test_take_from_container_valid_slot_boundary() {
        // Double chests have 54 slots (0-53); slot 53 must be accepted.
        let cmd = BotCommand::TakeFromContainer(53, 1);
        assert!(validate_command(&cmd).is_ok());
    }

    #[test]
    fn test_take_from_container_slot_too_high() {
        // Slot 54 is the first slot past the double-chest limit.
        let cmd = BotCommand::TakeFromContainer(54, 1);
        assert!(validate_command(&cmd).is_err());
        let cmd = BotCommand::TakeFromContainer(u8::MAX, 1);
        assert!(validate_command(&cmd).is_err());
    }

    #[test]
    fn test_take_from_container_slot_53_accepted() {
        // Explicit alias for the spec's required test name.
        let cmd = BotCommand::TakeFromContainer(53, 1);
        assert!(validate_command(&cmd).is_ok());
    }

    #[test]
    fn test_take_from_container_slot_54_rejected() {
        // Explicit alias for the spec's required test name.
        let cmd = BotCommand::TakeFromContainer(54, 1);
        assert!(validate_command(&cmd).is_err());
    }

    #[test]
    fn test_put_into_container_valid() {
        let cmd = BotCommand::PutIntoContainer(0, 64);
        assert!(validate_command(&cmd).is_ok());
    }

    #[test]
    fn test_put_into_container_zero_count() {
        let cmd = BotCommand::PutIntoContainer(0, 0);
        assert!(validate_command(&cmd).is_err());
    }

    #[test]
    fn test_put_into_container_valid_slot_boundary() {
        // Double chests: slot 53 must be accepted.
        let cmd = BotCommand::PutIntoContainer(53, 1);
        assert!(validate_command(&cmd).is_ok());
    }

    #[test]
    fn test_put_into_container_slot_too_high() {
        // Slot 54 is past the double-chest limit.
        let cmd = BotCommand::PutIntoContainer(54, 1);
        assert!(validate_command(&cmd).is_err());
        let cmd = BotCommand::PutIntoContainer(u8::MAX, 1);
        assert!(validate_command(&cmd).is_err());
    }

    // ── Error message quality ──────────────────────────────────────

    #[test]
    fn test_switch_slot_error_contains_context() {
        let cmd = BotCommand::SwitchHotbarSlot(255);
        let err = validate_command(&cmd).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("Hotbar slot"),
            "error should mention hotbar: {msg}"
        );
        assert!(
            msg.contains("255"),
            "error should contain invalid value: {msg}"
        );
    }

    #[test]
    fn test_position_error_contains_coordinate() {
        let cmd = BotCommand::MoveTo(BlockPos::new(0, 500, 0));
        let err = validate_command(&cmd).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("500"),
            "error should contain the invalid coordinate: {msg}"
        );
    }

    #[test]
    fn test_empty_message_error() {
        let cmd = BotCommand::SendChat(String::new());
        let err = validate_command(&cmd).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("empty"), "error should mention empty: {msg}");
    }

    #[test]
    fn test_radius_error_contains_value() {
        let cmd = BotCommand::QueryNearbyBlocks(0);
        let err = validate_command(&cmd).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("0"),
            "error should contain the invalid radius: {msg}"
        );
    }

    // ── v2 foundation variant validation ──────────────────────────

    #[test]
    fn test_smart_move_valid() {
        let cmd = BotCommand::SmartMove(BlockPos::new(10, 64, 20));
        assert!(validate_command(&cmd).is_ok());
    }

    #[test]
    fn test_smart_move_invalid_y() {
        let cmd = BotCommand::SmartMove(BlockPos::new(0, 500, 0));
        assert!(validate_command(&cmd).is_err());
    }

    #[test]
    fn test_fly_to_valid() {
        let cmd = BotCommand::FlyTo(BlockPos::new(0, 100, 0));
        assert!(validate_command(&cmd).is_ok());
    }

    #[test]
    fn test_fly_to_invalid_x() {
        let cmd = BotCommand::FlyTo(BlockPos::new(99_999_999, 64, 0));
        assert!(validate_command(&cmd).is_err());
    }

    #[test]
    fn test_collect_items_valid() {
        for radius in 1..=64u32 {
            let cmd = BotCommand::CollectItems(radius);
            assert!(
                validate_command(&cmd).is_ok(),
                "CollectItems radius {radius} should be valid"
            );
        }
    }

    #[test]
    fn test_collect_items_zero() {
        let cmd = BotCommand::CollectItems(0);
        assert!(validate_command(&cmd).is_err());
    }

    #[test]
    fn test_collect_items_too_large() {
        let cmd = BotCommand::CollectItems(65);
        assert!(validate_command(&cmd).is_err());
    }

    #[test]
    fn test_query_server_info_valid() {
        assert!(validate_command(&BotCommand::QueryServerInfo).is_ok());
    }

    #[test]
    fn test_query_chat_history_valid() {
        assert!(validate_command(&BotCommand::QueryChatHistory).is_ok());
    }

    #[test]
    fn test_query_world_view_valid() {
        for radius in 1..=32u8 {
            let cmd = BotCommand::QueryWorldView(radius);
            assert!(
                validate_command(&cmd).is_ok(),
                "QueryWorldView radius {radius} should be valid"
            );
        }
    }

    #[test]
    fn test_query_world_view_zero() {
        let cmd = BotCommand::QueryWorldView(0);
        assert!(validate_command(&cmd).is_err());
    }

    #[test]
    fn test_query_world_view_too_large() {
        // Values > 32 are rejected to keep the world view bounded.
        let cmd = BotCommand::QueryWorldView(33);
        assert!(validate_command(&cmd).is_err());
        let cmd = BotCommand::QueryWorldView(u8::MAX);
        assert!(validate_command(&cmd).is_err());
    }

    #[test]
    fn test_act_move_valid() {
        let cmd = BotCommand::Act(ActAction::Move {
            target: BlockPos::new(10, 64, 20),
        });
        assert!(validate_command(&cmd).is_ok());
    }

    #[test]
    fn test_act_smart_move_invalid() {
        let cmd = BotCommand::Act(ActAction::SmartMove {
            target: BlockPos::new(0, 500, 0),
        });
        assert!(validate_command(&cmd).is_err());
    }

    #[test]
    fn test_act_fly_valid() {
        let cmd = BotCommand::Act(ActAction::Fly {
            target: BlockPos::new(0, 200, 0),
        });
        assert!(validate_command(&cmd).is_ok());
    }

    #[test]
    fn test_act_mine_invalid() {
        let cmd = BotCommand::Act(ActAction::Mine {
            block_pos: BlockPos::new(0, -65, 0),
        });
        assert!(validate_command(&cmd).is_err());
    }

    #[test]
    fn test_act_attack_valid() {
        let cmd = BotCommand::Act(ActAction::Attack { entity_id: 42 });
        assert!(validate_command(&cmd).is_ok());
    }

    #[test]
    fn test_act_collect_items_zero() {
        let cmd = BotCommand::Act(ActAction::CollectItems { radius: 0 });
        assert!(validate_command(&cmd).is_err());
    }

    #[test]
    fn test_act_collect_items_valid() {
        let cmd = BotCommand::Act(ActAction::CollectItems { radius: 16 });
        assert!(validate_command(&cmd).is_ok());
    }

    // ── clamp_to_i32 (P0-#1) ───────────────────────────────────────

    /// Regression for P0-#1: a bare `v as i32` cast wraps values above
    /// `i32::MAX` into negative integers, which silently breaks the
    /// Chebyshev radius filter. `clamp_to_i32` must saturate instead.
    #[test]
    fn test_radius_clamp_overflow() {
        // 5_000_000_000 exceeds u32::MAX (4_294_967_295) so we use
        // `u32::MAX` directly to exercise the saturate branch. A bare
        // `v as i32` cast on `u32::MAX` wraps to -1, which silently
        // breaks the Chebyshev radius filter.
        assert_eq!(clamp_to_i32(u32::MAX), i32::MAX);
        assert_eq!(clamp_to_i32(4_000_000_000_u32), i32::MAX);
        assert_eq!(clamp_to_i32(100_u32), 100);
        assert_eq!(clamp_to_i32(i32::MAX as u32), i32::MAX);
        // u32::MAX must clamp (cannot wrap to -1).
        assert_eq!(clamp_to_i32(u32::MAX), i32::MAX);
        // Zero is a no-op.
        assert_eq!(clamp_to_i32(0), 0);
    }

    // ── validate_act_action (P0-#2) ────────────────────────────────

    /// Regression for P0-#2: handle_act must reject invalid `ActAction`
    /// payloads (out-of-range Y, oversized entity_id) before the bot
    /// executor sees them, so callers get a fast `InvalidParams` error
    /// rather than a wrapped internal failure.
    #[test]
    fn test_act_input_validation() {
        // Y far above the build height must be rejected.
        let bad_y = BotCommand::Act(ActAction::Move {
            target: BlockPos::new(0, 9999, 0),
        });
        assert!(
            matches!(validate_command(&bad_y), Err(BotError::InvalidParams(_))),
            "y=9999 should be rejected, got {:?}",
            validate_command(&bad_y)
        );

        // Y exactly at the boundary is allowed.
        let ok_y = BotCommand::Act(ActAction::Move {
            target: BlockPos::new(0, 64, 0),
        });
        assert!(validate_command(&ok_y).is_ok());

        // Y one below the lower build limit must be rejected.
        let low_y = BotCommand::Act(ActAction::Move {
            target: BlockPos::new(0, -65, 0),
        });
        assert!(matches!(
            validate_command(&low_y),
            Err(BotError::InvalidParams(_))
        ));

        // entity_id at the boundary (i32::MAX) is allowed.
        let ok_attack = BotCommand::Act(ActAction::Attack {
            entity_id: i32::MAX as u32,
        });
        assert!(validate_command(&ok_attack).is_ok());

        // entity_id above i32::MAX must be rejected.
        let bad_attack = BotCommand::Act(ActAction::Attack {
            entity_id: u32::MAX,
        });
        assert!(matches!(
            validate_command(&bad_attack),
            Err(BotError::InvalidParams(_))
        ));

        // The same checks apply to the public `validate_act_action` entry point.
        assert!(
            validate_act_action(&ActAction::Move {
                target: BlockPos::new(0, 9999, 0)
            })
            .is_err()
        );
        assert!(
            validate_act_action(&ActAction::Attack {
                entity_id: u32::MAX
            })
            .is_err()
        );
        assert!(
            validate_act_action(&ActAction::Move {
                target: BlockPos::new(0, 64, 0)
            })
            .is_ok()
        );
    }

    // ── Exhaustive match on all 34 variants ────────────────────────
    //
    // These tests provide compile-time coverage: if a new BotCommand variant
    // is added, the compiler will flag these matches as non-exhaustive.

    /// Count the number of variants — returns 1 for any valid command.
    /// Exists purely as a compile-time check that all variants are handled.
    #[allow(unreachable_code)]
    fn count_variants(cmd: &BotCommand) -> u32 {
        match cmd {
            BotCommand::MoveTo(_) => 1,
            BotCommand::WalkDirection(_, _) => 1,
            BotCommand::Jump => 1,
            BotCommand::Teleport(_) => 1,
            BotCommand::BreakBlock(_) => 1,
            BotCommand::PlaceBlock(_, _) => 1,
            BotCommand::UseItemOnBlock(_, _) => 1,
            BotCommand::SwitchHotbarSlot(_) => 1,
            BotCommand::DropItem(_, _) => 1,
            BotCommand::UseItem => 1,
            BotCommand::UseItemWithSlot(_) => 1,
            BotCommand::EquipTool(_) => 1,
            BotCommand::EquipToolWithMaterial(_, _) => 1,
            BotCommand::OpenContainer(_) => 1,
            BotCommand::TakeFromContainer(_, _) => 1,
            BotCommand::PutIntoContainer(_, _) => 1,
            BotCommand::CloseContainer => 1,
            BotCommand::AttackEntity(_) => 1,
            BotCommand::ShieldBlock(_) => 1,
            BotCommand::SendChat(_) => 1,
            BotCommand::ExecuteCommand(_) => 1,
            BotCommand::SetGameMode(_) => 1,
            BotCommand::QueryNearbyBlocks(_) => 1,
            BotCommand::QueryNearbyEntities(_) => 1,
            BotCommand::QuerySelfInfo => 1,
            BotCommand::QueryInventory => 1,
            BotCommand::QueryChunkSummary => 1,
            // ── v2 foundation variants ─────────────────────────────
            BotCommand::SmartMove(_) => 1,
            BotCommand::FlyTo(_) => 1,
            BotCommand::CollectItems(_) => 1,
            BotCommand::QueryServerInfo => 1,
            BotCommand::QueryChatHistory => 1,
            BotCommand::QueryWorldView(_) => 1,
            BotCommand::Act(_) => 1,
        }
    }

    #[test]
    fn test_all_variants_count_as_one() {
        let cmds = all_commands();
        for cmd in &cmds {
            assert_eq!(count_variants(cmd), 1);
        }
    }

    #[test]
    fn test_all_variants_pass_or_fail_validation() {
        let cmds = all_commands();
        for cmd in &cmds {
            // Every command must return either Ok or Err �?no panics.
            let _ = validate_command(cmd);
        }
    }

    fn all_commands() -> Vec<BotCommand> {
        vec![
            BotCommand::MoveTo(BlockPos::new(0, 0, 0)),
            BotCommand::WalkDirection(Direction::North, 1),
            BotCommand::Jump,
            BotCommand::Teleport(BlockPos::new(0, 0, 0)),
            BotCommand::BreakBlock(BlockPos::new(0, 0, 0)),
            BotCommand::PlaceBlock(BlockPos::new(0, 0, 0), "stone".into()),
            BotCommand::UseItemOnBlock(BlockPos::new(0, 0, 0), None),
            BotCommand::SwitchHotbarSlot(0),
            BotCommand::DropItem(0, 1),
            BotCommand::UseItem,
            BotCommand::UseItemWithSlot(0),
            BotCommand::EquipTool(ToolType::Pickaxe),
            BotCommand::EquipToolWithMaterial(ToolType::Pickaxe, MaterialTier::Diamond),
            BotCommand::OpenContainer(BlockPos::new(0, 0, 0)),
            BotCommand::TakeFromContainer(0, 1),
            BotCommand::PutIntoContainer(0, 1),
            BotCommand::CloseContainer,
            BotCommand::AttackEntity(0),
            BotCommand::ShieldBlock(true),
            BotCommand::SendChat("msg".into()),
            BotCommand::ExecuteCommand("/help".into()),
            BotCommand::SetGameMode(GameMode::Survival),
            BotCommand::QueryNearbyBlocks(10),
            BotCommand::QueryNearbyEntities(10),
            BotCommand::QuerySelfInfo,
            BotCommand::QueryInventory,
            BotCommand::QueryChunkSummary,
            // ── v2 foundation variants ─────────────────────────────
            BotCommand::SmartMove(BlockPos::new(0, 0, 0)),
            BotCommand::FlyTo(BlockPos::new(0, 0, 0)),
            BotCommand::CollectItems(8),
            BotCommand::QueryServerInfo,
            BotCommand::QueryChatHistory,
            BotCommand::QueryWorldView(4),
            BotCommand::Act(ActAction::Move {
                target: BlockPos::new(0, 0, 0),
            }),
        ]
    }
}
