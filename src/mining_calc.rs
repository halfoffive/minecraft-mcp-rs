//! Mining time calculations based on block hardness and tool.
//!
//! # Model scope (audit L-2)
//!
//! The formulas here model the base vanilla tool-speed mechanic only. The
//! vanilla **underwater ×5 penalty** (no Aqua Affinity helmet) and the
//! **airborne ×5 penalty** (not standing on the ground while mining) are
//! **NOT modeled** — the returned estimates are optimistic in those
//! conditions. When the bot mines underwater or while falling, the real
//! break time can be several times the estimate, and a caller that sleeps
//! `calculate_mine_time` before verifying the break (e.g.
//! `execute_mine_block`) may observe a timeout that the estimate did not
//! predict.

use crate::block_data::{
    ALT_TOOL_FOR_BLOCK, BLOCK_HARDNESS, BLOCK_TO_TOOL_TYPE, HARVEST_LEVEL, MATERIAL_TIER_SPEED,
    harvest_level_of, requires_tool_for_drops,
};
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
    if tool_type == expected {
        return true;
    }
    // Cobweb-class blocks accept a second tool at the same vanilla speed
    // (2026-08-30 review).
    ALT_TOOL_FOR_BLOCK.get(block_type).copied() == Some(tool_type)
}

/// Checks whether the given block genuinely requires a specific (non-Hand)
/// tool to mine it efficiently.
///
/// The 5× wrong-tool penalty applies only to blocks that genuinely require a
/// tool (audit L-1). Vanilla's rule: a block "requires a tool" when mining
/// it without the right tool never drops the block — either because it has a
/// harvest level above 0 in HARVEST_LEVEL, or because it carries vanilla's
/// requires_correct_tool_for_drops flag (see requires_tool_for_drops — e.g.
/// cobbled_deepslate is tier 0 but drops nothing by hand). Blocks like dirt,
/// sand, and logs ARE in BLOCK_TO_TOOL_TYPE (they have a fastest tool) but
/// are legal hand mines, so they incur NO wrong-tool penalty. stone has
/// level 1 per the project's conservative convention, so its ×5 stays.
///
/// Returns false for unknown blocks (treated as not requiring a tool,
/// preserving the semantics of is_correct_tool which considers Hand correct
/// for unknown blocks).
pub fn block_requires_tool(block_type: &str) -> bool {
    if requires_tool_for_drops(block_type) {
        return true;
    }
    HARVEST_LEVEL
        .get(block_type)
        .copied()
        .map(|level| level > 0)
        .unwrap_or(false)
}

/// Calculates the time (in seconds) required to mine a block.
///
/// This mirrors the vanilla tick-damage model instead of applying the 5×
/// penalty as a multiplier on top of the correct-tool formula (audit F-7):
///
/// - effective (can-harvest) breaking uses 30 ticks per hardness point
///   (`hardness * 1.5 / tool_speed`);
/// - non-harvest breaking uses 100 ticks per hardness point
///   (`hardness * 5.0 / speed`), where `speed` is `1.0` unless the tool is
///   both the correct type AND strong enough to harvest the block.
///
/// - Unbreakable blocks (hardness < 0) return [`f64::INFINITY`].
/// - Hand speed is always `1.0`.
/// - The wrong tool (including Hand) never gets the tool speed and, when the
///   block genuinely requires a tool ([`block_requires_tool`]), uses the
///   100-tick branch. Blocks that don't require a tool (e.g. dirt, sand,
///   unknown blocks) still use the 30-tick branch with speed `1.0` for a
///   non-matching tool (audit L-1).
/// - A correct tool whose material tier is below the block's harvest level
///   (e.g. a wooden pickaxe on iron ore) uses the non-harvest branch too —
///   it yields no drop — but KEEPS its tool speed: vanilla gates only the
///   tick branch on the harvest level, not the speed (2026-08-30 review).
///   Callers that need a guaranteed drop must still filter by harvest level
///   before mining; this function only models break time.
///
/// Estimate scope (audit L-2): vanilla's underwater ×5 (no Aqua Affinity)
/// and airborne ×5 penalties are NOT modeled — the returned time is
/// optimistic in those conditions.
pub fn calculate_mine_time(block_type: &str, tool_type: ToolType, material: MaterialTier) -> f64 {
    let hardness = get_block_hardness(block_type);

    // Unbreakable blocks (bedrock, etc.)
    if hardness < 0.0 {
        return f64::INFINITY;
    }

    let correct_tool = is_correct_tool(tool_type, block_type);
    let required_level = HARVEST_LEVEL.get(block_type).copied().unwrap_or(0);
    let tool_level = harvest_level_of(material);

    // Vanilla applies the tool speed whenever the tool TYPE matches — the
    // harvest level does NOT gate the speed, only the 30/100-tick branch
    // below (2026-08-30 review: the old model priced an under-tier correct
    // tool at hand speed, doubling the estimate; e.g. a wooden pickaxe on
    // iron_ore takes 7.5 s in vanilla, not 15 s). Hand always mines at 1.0.
    // `required_level` defaults to 0, so a correct tool on a tier-0 block
    // (e.g. any pickaxe on cobbled_deepslate) keeps its speed.
    let speed_applies = tool_type != ToolType::Hand && correct_tool;
    let speed = if speed_applies {
        *MATERIAL_TIER_SPEED.get(&material).unwrap_or(&1.0)
    } else {
        1.0
    };

    // Vanilla breaks a block with 30 ticks of progress per hardness point
    // when the item can harvest it, and 100 ticks otherwise. A block that
    // does not require a tool is always harvestable, even with the wrong
    // tool (dirt with a pickaxe), so it stays on the 30-tick branch.
    let can_harvest = tool_type != ToolType::Hand && correct_tool && tool_level >= required_level;
    let requires_tool = block_requires_tool(block_type);
    let ticks_per_hardness = if !requires_tool || can_harvest {
        30.0
    } else {
        100.0
    };

    hardness * ticks_per_hardness / 20.0 / speed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{MaterialTier, ToolType};

    // ── get_block_hardness ──────────────────────────────────

    #[test]
    fn test_get_block_hardness_known() {
        assert!((get_block_hardness("stone") - 1.5).abs() < 1e-9);
        assert!((get_block_hardness("obsidian") - 50.0).abs() < 1e-9);
        assert!((get_block_hardness("dirt") - 0.5).abs() < 1e-9);
    }

    #[test]
    fn test_get_block_hardness_unbreakable() {
        assert!((get_block_hardness("bedrock") - (-1.0)).abs() < 1e-9);
    }

    #[test]
    fn test_get_block_hardness_unknown_defaults_to_one() {
        assert!((get_block_hardness("unknown_block") - 1.0).abs() < 1e-9);
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
        assert!((time - 0.375).abs() < 1e-9); // 1.5 * 1.5 / 6.0 = 0.375
    }

    // L-1 (audit): `test_mine_time_stone_with_iron_axe_wrong_tool` (stone's
    // 5× penalty stays intact after the L-1 gate) is asserted below in the
    // L-1 section alongside the new no-penalty block tests.

    #[test]
    fn test_mine_time_obsidian_with_diamond() {
        // obsidian = 50.0, diamond speed = 8.0
        let time = calculate_mine_time("obsidian", ToolType::Pickaxe, MaterialTier::Diamond);
        assert!((time - 9.375).abs() < 1e-9); // 50.0 * 1.5 / 8.0 = 9.375
    }

    #[test]
    fn test_mine_time_unknown_block_hand() {
        // default hardness 1.0, hand speed = 1.0, no penalty
        let time = calculate_mine_time("unknown_block", ToolType::Hand, MaterialTier::Wood);
        assert!((time - 1.5).abs() < 1e-9); // 1.0 * 1.5 / 1.0 = 1.5
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
        assert!((time - 0.0625).abs() < 1e-9); // 0.5 * 1.5 / 12.0 = 0.0625
    }

    #[test]
    fn test_mine_time_hand_no_penalty() {
        // Hand on ice: no tool-for-drops gate (ice has HARVEST_LEVEL 0 and
        // is NOT in TOOL_REQUIRED_FOR_DROPS — the pickaxe entry in
        // BLOCK_TO_TOOL_TYPE only makes the pickaxe faster, it does not
        // gate drops), so Hand stays on the 30-tick branch with speed 1.0.
        let time = calculate_mine_time("ice", ToolType::Hand, MaterialTier::Wood);
        assert!((time - 0.75).abs() < 1e-9); // 0.5 * 1.5 / 1.0 = 0.75
    }

    #[test]
    fn test_mine_time_hand_on_tool_required() {
        // F-7: non-harvest mining is the independent 100-tick branch
        // (hardness * 5 / speed), NOT the correct-tool formula with a ×5
        // multiplier stacked on top (which would give 11.25s).
        let time = calculate_mine_time("stone", ToolType::Hand, MaterialTier::Wood);
        assert!((time - 7.5).abs() < 1e-9); // 1.5 * 5.0 / 1.0 = 7.5
    }

    #[test]
    fn test_mine_time_sword_is_wrong_tool() {
        // sword on stone: wrong tool, non-harvest branch
        let time = calculate_mine_time("stone", ToolType::Sword, MaterialTier::Iron);
        assert!((time - 7.5).abs() < 1e-9); // 1.5 * 5.0 / 1.0 = 7.5
    }

    #[test]
    fn test_mine_time_unknown_block_wrong_tool() {
        // unknown block defaults to Hand and doesn't require a tool, wrong tool
        // gets no speed bonus (same as hand), no penalty.
        let time = calculate_mine_time("unknown_block", ToolType::Pickaxe, MaterialTier::Iron);
        // hardness 1.0 * 1.5 / 1.0 * 1.0 = 1.5
        assert!((time - 1.5).abs() < 1e-9);
    }

    #[test]
    fn test_mine_time_wrong_tool_speed_equals_hand() {
        let axe_time = calculate_mine_time("stone", ToolType::Axe, MaterialTier::Iron);
        let hand_time = calculate_mine_time("stone", ToolType::Hand, MaterialTier::Wood);
        assert!((axe_time - hand_time).abs() < 1e-9);
        assert!((axe_time - 7.5).abs() < 1e-9);
    }

    #[test]
    fn test_mine_time_correct_tool_still_gets_speed() {
        let time = calculate_mine_time("stone", ToolType::Pickaxe, MaterialTier::Iron);
        assert!((time - 0.375).abs() < 1e-9);
    }

    // ── L-1 (audit): wrong-tool penalty only for tool-requiring blocks ──

    /// L-1 (audit): DIRT does not genuinely require a tool (vanilla harvest
    /// level 0 — hand mining drops the block). The old `block_requires_tool`
    /// returned true for ANY block in `BLOCK_TO_TOOL_TYPE`, so an iron
    /// pickaxe on dirt got the 5× wrong-tool penalty (3.75s instead of the
    /// vanilla 0.75s). Vanilla applies the ×5 only when the block genuinely
    /// requires a tool; dirt must NOT be penalised.
    #[test]
    fn test_mine_time_dirt_with_pickaxe_no_penalty() {
        // dirt hardness 0.5, wrong tool (pickaxe) speed 1.0, NO penalty:
        // 0.5 * 1.5 / 1.0 = 0.75s — not 3.75s.
        let time = calculate_mine_time("dirt", ToolType::Pickaxe, MaterialTier::Iron);
        assert!(
            (time - 0.75).abs() < 1e-9,
            "dirt with iron pickaxe must be 0.75s (no wrong-tool penalty), got {time}"
        );
    }

    /// L-1 (audit): sand is likewise a level-0 block — wrong tool, no ×5.
    #[test]
    fn test_mine_time_sand_with_sword_no_penalty() {
        // sand hardness 0.5, wrong tool (sword) speed 1.0, no penalty: 0.75s.
        let time = calculate_mine_time("sand", ToolType::Sword, MaterialTier::Iron);
        assert!(
            (time - 0.75).abs() < 1e-9,
            "sand with sword must be 0.75s (no wrong-tool penalty), got {time}"
        );
    }

    /// L-1 (audit): logs are level-0 blocks — wrong tool (pickaxe), no ×5.
    #[test]
    fn test_mine_time_oak_log_with_pickaxe_no_penalty() {
        // oak_log hardness 2.0, wrong tool speed 1.0, no penalty: 3.0s.
        let time = calculate_mine_time("oak_log", ToolType::Pickaxe, MaterialTier::Iron);
        assert!(
            (time - 3.0).abs() < 1e-9,
            "oak_log with pickaxe must be 3.0s (no wrong-tool penalty), got {time}"
        );
    }

    /// L-1 (audit): STONE genuinely requires a tool (project convention:
    /// harvest level 1) — the wrong tool still uses the slow non-harvest
    /// branch (F-7 corrected the formula from the inflated 11.25s to the
    /// vanilla 7.5s).
    #[test]
    fn test_mine_time_stone_with_iron_axe_wrong_tool() {
        // stone hardness 1.5, wrong tool = speed 1.0, non-harvest branch:
        // 1.5 * 5.0 / 1.0 = 7.5 — unchanged by the L-1 gate.
        let time = calculate_mine_time("stone", ToolType::Axe, MaterialTier::Iron);
        assert!(
            (time - 7.5).abs() < 1e-9,
            "stone with iron axe must use the 100-tick branch (7.5s), got {time}"
        );
    }

    /// F-20 (updated 2026-08-30 review): a correct tool whose material tier
    /// is below the block's harvest level cannot HARVEST the block (100-tick
    /// branch) but KEEPS its tool speed — vanilla gates only the tick branch
    /// on the harvest level. A wooden pickaxe (speed 2.0) on iron_ore
    /// (hardness 3.0) therefore takes 3.0 * 5.0 / 2.0 = 7.5 s, not the
    /// 15 s the old hand-speed model claimed.
    #[test]
    fn test_mine_time_correct_tool_insufficient_harvest_level() {
        let hardness = get_block_hardness("iron_ore");
        let time = calculate_mine_time("iron_ore", ToolType::Pickaxe, MaterialTier::Wood);
        let expected = hardness * 5.0 / 2.0;
        assert!(
            (time - expected).abs() < 1e-9,
            "wooden pickaxe on iron_ore must use the non-harvest branch at tool speed ({expected}s), got {time}"
        );

        // An iron pickaxe (level 2) harvests iron_ore at its material speed.
        let harvested = calculate_mine_time("iron_ore", ToolType::Pickaxe, MaterialTier::Iron);
        let expected_harvest = hardness * 1.5 / 6.0;
        assert!((harvested - expected_harvest).abs() < 1e-9);
    }

    #[test]
    fn test_is_correct_tool_accepts_alt_tool() {
        // 2026-08-30 review: cobweb accepts shears alongside the sword
        // primary (same vanilla break speed).
        assert!(is_correct_tool(ToolType::Sword, "cobweb"));
        assert!(is_correct_tool(ToolType::Shears, "cobweb"));
        assert!(!is_correct_tool(ToolType::Pickaxe, "cobweb"));
        // The alt mechanism must not leak to other blocks.
        assert!(!is_correct_tool(ToolType::Shears, "stone"));
        // Hand-on-unknown stays correct.
        assert!(is_correct_tool(ToolType::Hand, "unknown_block"));
    }
}
