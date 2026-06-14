//! Mining time calculations based on block hardness and tool.

use crate::block_data::{BLOCK_HARDNESS, BLOCK_TO_TOOL_TYPE, MATERIAL_TIER_SPEED};
use crate::types::{MaterialTier, ToolType};

/// Looks up the hardness value for a given block type.
///
/// Returns `1.0` for unknown blocks.
pub fn get_block_hardness(block_type: &str) -> f64 {
    *BLOCK_HARDNESS.get(block_type).unwrap_or(&1.0)
}

/// Checks whether the given tool is the correct type for mining the block.
///
/// Unknown blocks default to [`ToolType::Hand`], so [`ToolType::Hand`] is
/// considered correct for unknown blocks.
pub fn is_correct_tool(tool_type: ToolType, block_type: &str) -> bool {
    let expected = BLOCK_TO_TOOL_TYPE
        .get(block_type)
        .copied()
        .unwrap_or(ToolType::Hand);
    tool_type == expected
}

/// Calculates the time (in seconds) required to mine a block.
///
/// Formula: `hardness * 1.5 / tool_speed * penalty`
///
/// - Unbreakable blocks (hardness < 0) return [`f64::INFINITY`].
/// - Wrong tool penalty: 5× multiplier.
/// - Hand speed is always `1.0` and incurs no penalty.
pub fn calculate_mine_time(block_type: &str, tool_type: ToolType, material: MaterialTier) -> f64 {
    let hardness = get_block_hardness(block_type);

    // Unbreakable blocks (bedrock, etc.)
    if hardness < 0.0 {
        return f64::INFINITY;
    }

    let speed = if tool_type == ToolType::Hand {
        1.0
    } else {
        *MATERIAL_TIER_SPEED.get(&material).unwrap_or(&1.0)
    };

    let penalty = if tool_type != ToolType::Hand && !is_correct_tool(tool_type, block_type) {
        5.0
    } else {
        1.0
    };

    hardness * 1.5 / speed * penalty
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{MaterialTier, ToolType};

    // ── get_block_hardness ──────────────────────────────────

    #[test]
    fn test_get_block_hardness_known() {
        assert!((get_block_hardness("stone") - 1.5).abs() < f64::EPSILON);
        assert!((get_block_hardness("obsidian") - 50.0).abs() < f64::EPSILON);
        assert!((get_block_hardness("dirt") - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_get_block_hardness_unbreakable() {
        assert!((get_block_hardness("bedrock") - (-1.0)).abs() < f64::EPSILON);
    }

    #[test]
    fn test_get_block_hardness_unknown_defaults_to_one() {
        assert!((get_block_hardness("unknown_block") - 1.0).abs() < f64::EPSILON);
    }

    // ── is_correct_tool ─────────────────────────────────────

    #[test]
    fn test_is_correct_tool_pickaxe_for_stone() {
        assert!(is_correct_tool(ToolType::Pickaxe, "stone"));
        assert!(is_correct_tool(ToolType::Pickaxe, "iron_ore"));
        assert!(is_correct_tool(ToolType::Pickaxe, "obsidian"));
    }

    #[test]
    fn test_is_correct_tool_axe_for_wood() {
        assert!(is_correct_tool(ToolType::Axe, "oak_log"));
        assert!(is_correct_tool(ToolType::Axe, "crafting_table"));
    }

    #[test]
    fn test_is_correct_tool_shovel_for_dirt() {
        assert!(is_correct_tool(ToolType::Shovel, "dirt"));
        assert!(is_correct_tool(ToolType::Shovel, "sand"));
    }

    #[test]
    fn test_is_correct_tool_shears_for_leaves() {
        assert!(is_correct_tool(ToolType::Shears, "oak_leaves"));
        assert!(is_correct_tool(ToolType::Shears, "white_wool"));
    }

    #[test]
    fn test_is_correct_tool_hand_for_unknown() {
        assert!(is_correct_tool(ToolType::Hand, "unknown_block"));
    }

    #[test]
    fn test_is_correct_tool_wrong_tool() {
        assert!(!is_correct_tool(ToolType::Axe, "stone"));
        assert!(!is_correct_tool(ToolType::Pickaxe, "oak_log"));
        assert!(!is_correct_tool(ToolType::Shovel, "stone"));
        assert!(!is_correct_tool(ToolType::Hand, "stone"));
    }

    // ── calculate_mine_time ─────────────────────────────────

    #[test]
    fn test_mine_time_stone_with_iron_pickaxe() {
        // stone hardness = 1.5, iron speed = 6.0, correct tool
        let time = calculate_mine_time("stone", ToolType::Pickaxe, MaterialTier::Iron);
        assert!((time - 0.375).abs() < f64::EPSILON); // 1.5 * 1.5 / 6.0 = 0.375
    }

    #[test]
    fn test_mine_time_stone_with_iron_axe_wrong_tool() {
        // stone hardness = 1.5, iron speed = 6.0, wrong tool = 5×
        let time = calculate_mine_time("stone", ToolType::Axe, MaterialTier::Iron);
        assert!((time - 1.875).abs() < f64::EPSILON); // 1.5 * 1.5 / 6.0 * 5 = 1.875
    }

    #[test]
    fn test_mine_time_obsidian_with_diamond() {
        // obsidian = 50.0, diamond speed = 8.0
        let time = calculate_mine_time("obsidian", ToolType::Pickaxe, MaterialTier::Diamond);
        assert!((time - 9.375).abs() < f64::EPSILON); // 50.0 * 1.5 / 8.0 = 9.375
    }

    #[test]
    fn test_mine_time_unknown_block_hand() {
        // default hardness 1.0, hand speed = 1.0, no penalty
        let time = calculate_mine_time("unknown_block", ToolType::Hand, MaterialTier::Wood);
        assert!((time - 1.5).abs() < f64::EPSILON); // 1.0 * 1.5 / 1.0 = 1.5
    }

    #[test]
    fn test_mine_time_unbreakable() {
        let time = calculate_mine_time("bedrock", ToolType::Pickaxe, MaterialTier::Netherite);
        assert_eq!(time, f64::INFINITY);
    }

    #[test]
    fn test_mine_time_gold_max_speed() {
        // dirt = 0.5, gold speed = 12.0
        let time = calculate_mine_time("dirt", ToolType::Shovel, MaterialTier::Gold);
        assert!((time - 0.0625).abs() < f64::EPSILON); // 0.5 * 1.5 / 12.0 = 0.0625
    }

    #[test]
    fn test_mine_time_hand_no_penalty() {
        // stone with hand: no penalty, speed = 1.0
        let time = calculate_mine_time("stone", ToolType::Hand, MaterialTier::Wood);
        assert!((time - 2.25).abs() < f64::EPSILON); // 1.5 * 1.5 / 1.0 = 2.25
    }

    #[test]
    fn test_mine_time_sword_is_wrong_tool() {
        // sword on stone: wrong tool, gets penalty
        let time = calculate_mine_time("stone", ToolType::Sword, MaterialTier::Iron);
        // 1.5 * 1.5 / 6.0 * 5.0 = 1.875
        assert!((time - 1.875).abs() < f64::EPSILON);
    }

    #[test]
    fn test_mine_time_unknown_block_wrong_tool() {
        // unknown block defaults to Hand, so any non-Hand tool is "wrong"
        let time = calculate_mine_time("unknown_block", ToolType::Pickaxe, MaterialTier::Iron);
        // hardness 1.0 * 1.5 / 6.0 * 5.0 = 1.25
        assert!((time - 1.25).abs() < f64::EPSILON);
    }
}
