//! Input validation for commands and tool parameters.
//!
//! This module provides validation functions that check command parameters
//! for correctness before they are passed to the bot engine. Validation is
//! stateless — it only checks that the values themselves are within expected
//! ranges, without consulting any external state.

use crate::error::BotError;
use crate::types::{BlockPos, BotCommand};

// ── World bounds constants ────────────────────────────────────────────────

/// Minecraft world border half-extent on X and Z axes.
const WORLD_BORDER: i32 = 30_000_000;

/// Minimum build height (Y level).
const MIN_Y: i32 = -64;

/// Maximum build height (Y level).
const MAX_Y: i32 = 320;

// ── Public validation API ─────────────────────────────────────────────────

/// Validate a [`BotCommand`] before execution.
///
/// Returns `Ok(())` if all parameters are within acceptable ranges, or
/// `Err(BotError)` describing the first validation failure.
///
/// # Errors
///
/// Returns [`BotError::Internal`] when any parameter is out of bounds or
/// otherwise invalid.
pub fn validate_command(cmd: &BotCommand) -> Result<(), BotError> {
    match cmd {
        // Position-based commands: all validate position bounds
        BotCommand::MoveTo(pos)
        | BotCommand::Teleport(pos)
        | BotCommand::BreakBlock(pos)
        | BotCommand::PlaceBlock(pos, _)
        | BotCommand::UseItemOnBlock(pos)
        | BotCommand::OpenContainer(pos) => validate_position(pos),

        // Direction is verified by the type system — no runtime checks needed.
        BotCommand::WalkDirection(_) => Ok(()),

        // Parameterless commands — always valid.
        BotCommand::Jump
        | BotCommand::UseItem
        | BotCommand::EquipTool(_)
        | BotCommand::CloseContainer
        | BotCommand::AttackEntity(_)
        | BotCommand::ShieldBlock
        | BotCommand::SetGameMode(_)
        | BotCommand::QuerySelfInfo
        | BotCommand::QueryInventory
        | BotCommand::QueryChunkSummary => Ok(()),

        // Hotbar slot must be in range 0-8.
        BotCommand::SwitchHotbarSlot(slot) => {
            if *slot > 8 {
                return Err(BotError::Internal(format!(
                    "Hotbar slot must be 0-8, got {slot}"
                )));
            }
            Ok(())
        }

        // Slotted operations require a positive count.
        BotCommand::DropItem(_, count)
        | BotCommand::TakeFromContainer(_, count)
        | BotCommand::PutIntoContainer(_, count) => {
            if *count == 0 {
                return Err(BotError::Internal(
                    "Count must be greater than 0".into(),
                ));
            }
            Ok(())
        }

        // Messages must be non-empty (whitespace-only also rejected).
        BotCommand::SendChat(msg) | BotCommand::ExecuteCommand(msg) => {
            if msg.trim().is_empty() {
                return Err(BotError::Internal(
                    "Message cannot be empty".into(),
                ));
            }
            Ok(())
        }

        // Block queries use a capped radius; entity queries only require > 0.
        BotCommand::QueryNearbyBlocks(radius) => {
            if *radius < 1 || *radius > 64 {
                return Err(BotError::Internal(format!(
                    "Radius must be between 1 and 64, got {radius}"
                )));
            }
            Ok(())
        }

        BotCommand::QueryNearbyEntities(radius) => {
            if *radius == 0 {
                return Err(BotError::Internal(format!(
                    "Radius must be greater than 0, got {radius}"
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
pub fn validate_coordinates(x: i32, y: i32, z: i32) -> Result<(), String> {
    if x.abs() > WORLD_BORDER {
        return Err(format!(
            "x coordinate {x} out of range (must be between -{WORLD_BORDER} and {WORLD_BORDER})"
        ));
    }
    if z.abs() > WORLD_BORDER {
        return Err(format!(
            "z coordinate {z} out of range (must be between -{WORLD_BORDER} and {WORLD_BORDER})"
        ));
    }
    if y < MIN_Y || y > MAX_Y {
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
        return Err(BotError::Internal(format!(
            "X coordinate {} out of bounds (must be between {} and {})",
            pos.x,
            -WORLD_BORDER,
            WORLD_BORDER
        )));
    }
    if pos.y < MIN_Y || pos.y > MAX_Y {
        return Err(BotError::Internal(format!(
            "Y coordinate {} out of bounds (must be between {MIN_Y} and {MAX_Y})",
            pos.y,
        )));
    }
    if pos.z < -WORLD_BORDER || pos.z > WORLD_BORDER {
        return Err(BotError::Internal(format!(
            "Z coordinate {} out of bounds (must be between {} and {})",
            pos.z,
            -WORLD_BORDER,
            WORLD_BORDER
        )));
    }
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Direction, GameMode, ToolType};

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
        let cmd = BotCommand::UseItemOnBlock(BlockPos::new(0, 0, 0));
        assert!(validate_command(&cmd).is_ok());
    }

    #[test]
    fn test_use_item_on_block_invalid_x() {
        let cmd = BotCommand::UseItemOnBlock(BlockPos::new(30_000_001, 0, 0));
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
        for radius in 1..=64u32 {
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
        let cmd = BotCommand::QueryNearbyBlocks(65);
        assert!(validate_command(&cmd).is_err());
    }

    #[test]
    fn test_query_nearby_entities_valid() {
        let cmd = BotCommand::QueryNearbyEntities(1);
        assert!(validate_command(&cmd).is_ok());
    }

    #[test]
    fn test_query_nearby_entities_zero() {
        let cmd = BotCommand::QueryNearbyEntities(0);
        assert!(validate_command(&cmd).is_err());
    }

    #[test]
    fn test_query_nearby_entities_large() {
        // Entity queries have no upper bound per spec — only must be > 0.
        let cmd = BotCommand::QueryNearbyEntities(999);
        assert!(validate_command(&cmd).is_ok());
    }

    // ── Pass-through commands (always valid) ───────────────────────

    #[test]
    fn test_jump_valid() {
        assert!(validate_command(&BotCommand::Jump).is_ok());
    }

    #[test]
    fn test_walk_direction_valid() {
        let cmd = BotCommand::WalkDirection(Direction::North);
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
        assert!(validate_command(&BotCommand::ShieldBlock).is_ok());
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
    fn test_put_into_container_valid() {
        let cmd = BotCommand::PutIntoContainer(0, 64);
        assert!(validate_command(&cmd).is_ok());
    }

    #[test]
    fn test_put_into_container_zero_count() {
        let cmd = BotCommand::PutIntoContainer(0, 0);
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
        assert!(
            msg.contains("empty"),
            "error should mention empty: {msg}"
        );
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

    // ── Exhaustive match on all 25 variants ────────────────────────
    //
    // These tests provide compile-time coverage: if a new BotCommand variant
    // is added, the compiler will flag these matches as non-exhaustive.

    /// Count the number of variants — returns 1 for any valid command.
    /// Exists purely as a compile-time check that all variants are handled.
    #[allow(unreachable_code)]
    fn count_variants(cmd: &BotCommand) -> u32 {
        match cmd {
            BotCommand::MoveTo(_) => 1,
            BotCommand::WalkDirection(_) => 1,
            BotCommand::Jump => 1,
            BotCommand::Teleport(_) => 1,
            BotCommand::BreakBlock(_) => 1,
            BotCommand::PlaceBlock(_, _) => 1,
            BotCommand::UseItemOnBlock(_) => 1,
            BotCommand::SwitchHotbarSlot(_) => 1,
            BotCommand::DropItem(_, _) => 1,
            BotCommand::UseItem => 1,
            BotCommand::EquipTool(_) => 1,
            BotCommand::OpenContainer(_) => 1,
            BotCommand::TakeFromContainer(_, _) => 1,
            BotCommand::PutIntoContainer(_, _) => 1,
            BotCommand::CloseContainer => 1,
            BotCommand::AttackEntity(_) => 1,
            BotCommand::ShieldBlock => 1,
            BotCommand::SendChat(_) => 1,
            BotCommand::ExecuteCommand(_) => 1,
            BotCommand::SetGameMode(_) => 1,
            BotCommand::QueryNearbyBlocks(_) => 1,
            BotCommand::QueryNearbyEntities(_) => 1,
            BotCommand::QuerySelfInfo => 1,
            BotCommand::QueryInventory => 1,
            BotCommand::QueryChunkSummary => 1,
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
            // Every command must return either Ok or Err — no panics.
            let _ = validate_command(cmd);
        }
    }

    fn all_commands() -> Vec<BotCommand> {
        vec![
            BotCommand::MoveTo(BlockPos::new(0, 0, 0)),
            BotCommand::WalkDirection(Direction::North),
            BotCommand::Jump,
            BotCommand::Teleport(BlockPos::new(0, 0, 0)),
            BotCommand::BreakBlock(BlockPos::new(0, 0, 0)),
            BotCommand::PlaceBlock(BlockPos::new(0, 0, 0), "stone".into()),
            BotCommand::UseItemOnBlock(BlockPos::new(0, 0, 0)),
            BotCommand::SwitchHotbarSlot(0),
            BotCommand::DropItem(0, 1),
            BotCommand::UseItem,
            BotCommand::EquipTool(ToolType::Pickaxe),
            BotCommand::OpenContainer(BlockPos::new(0, 0, 0)),
            BotCommand::TakeFromContainer(0, 1),
            BotCommand::PutIntoContainer(0, 1),
            BotCommand::CloseContainer,
            BotCommand::AttackEntity(0),
            BotCommand::ShieldBlock,
            BotCommand::SendChat("msg".into()),
            BotCommand::ExecuteCommand("/help".into()),
            BotCommand::SetGameMode(GameMode::Survival),
            BotCommand::QueryNearbyBlocks(10),
            BotCommand::QueryNearbyEntities(10),
            BotCommand::QuerySelfInfo,
            BotCommand::QueryInventory,
            BotCommand::QueryChunkSummary,
        ]
    }
}
