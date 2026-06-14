//! Block and tool data tables for the Minecraft MCP server.
//!
//! Provides lookup functions for block-to-tool mappings, material tier speeds,
//! tool name patterns, block hardness values, and mining time calculations.
//!
//! > **Note:** Most items in this module are lookup tables designed for the
//! > bot ops layer.  They are retained for the integration plan.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::LazyLock;

use crate::types::{MaterialTier, ToolType};

/// An item stack in an inventory slot.
#[derive(Debug, Clone)]
pub struct ItemStack {
    pub item_id: String,
    pub count: u8,
}

/// Maps block types to the tool type required to mine them efficiently.
///
/// Unknown blocks default to [`ToolType::Hand`].
pub static BLOCK_TO_TOOL_TYPE: LazyLock<HashMap<&'static str, ToolType>> = LazyLock::new(|| {
    let mut m = HashMap::new();

    // --- Pickaxe blocks ---
    for &block in &[
        "stone",
        "cobblestone",
        "andesite",
        "diorite",
        "granite",
        "stone_bricks",
        "mossy_stone_bricks",
        "cracked_stone_bricks",
        "stone_slab",
        "cobblestone_slab",
        "stone_stairs",
        "cobblestone_stairs",
        "cobblestone_wall",
        "bedrock",
        "obsidian",
        // Ores
        "coal_ore",
        "iron_ore",
        "gold_ore",
        "diamond_ore",
        "emerald_ore",
        "lapis_ore",
        "redstone_ore",
        "copper_ore",
        "deepslate_coal_ore",
        "deepslate_iron_ore",
        "deepslate_gold_ore",
        "deepslate_diamond_ore",
        "deepslate_emerald_ore",
        "deepslate_lapis_ore",
        "deepslate_redstone_ore",
        "deepslate_copper_ore",
        "deepslate",
        "tuff",
        "calcite",
        // Nether / End
        "netherrack",
        "nether_quartz_ore",
        "nether_gold_ore",
        "end_stone",
        "purpur_block",
        "purpur_pillar",
        // Manufactured
        "bricks",
        "brick_slab",
        "brick_stairs",
        "iron_block",
        "gold_block",
        "diamond_block",
        "emerald_block",
        "furnace",
        "blast_furnace",
        "smoker",
        "anvil",
        "chipped_anvil",
        "damaged_anvil",
        "enchanting_table",
        "brewing_stand",
        "hopper",
        "dropper",
        "dispenser",
        "observer",
        "chest",
        "trapped_chest",
        "ender_chest",
    ] {
        m.insert(block, ToolType::Pickaxe);
    }

    // --- Axe blocks ---
    for &block in &[
        "oak_log",
        "spruce_log",
        "birch_log",
        "jungle_log",
        "acacia_log",
        "dark_oak_log",
        "oak_planks",
        "spruce_planks",
        "birch_planks",
        "jungle_planks",
        "acacia_planks",
        "dark_oak_planks",
        "oak_stairs",
        "spruce_stairs",
        "birch_stairs",
        "oak_slab",
        "spruce_slab",
        "birch_slab",
        "oak_fence",
        "spruce_fence",
        "birch_fence",
        "oak_fence_gate",
        "oak_door",
        "spruce_door",
        "birch_door",
        "crafting_table",
        "bookshelf",
        "ladder",
        "barrel",
    ] {
        m.insert(block, ToolType::Axe);
    }

    // --- Shovel blocks ---
    for &block in &[
        "dirt",
        "grass_block",
        "dirt_path",
        "coarse_dirt",
        "rooted_dirt",
        "sand",
        "red_sand",
        "suspicious_sand",
        "gravel",
        "clay",
        "farmland",
        "soul_sand",
        "soul_soil",
        "snow",
        "snow_block",
        "powder_snow",
        "mud",
        "muddy_mangrove_roots",
        "mycelium",
        "podzol",
    ] {
        m.insert(block, ToolType::Shovel);
    }

    // --- Shears blocks ---
    for &block in &[
        "oak_leaves",
        "spruce_leaves",
        "birch_leaves",
        "jungle_leaves",
        "acacia_leaves",
        "dark_oak_leaves",
        "azalea_leaves",
        "white_wool",
        "orange_wool",
        "magenta_wool",
        "light_blue_wool",
        "yellow_wool",
        "lime_wool",
        "pink_wool",
        "gray_wool",
        "light_gray_wool",
        "cyan_wool",
        "purple_wool",
        "blue_wool",
        "brown_wool",
        "green_wool",
        "red_wool",
        "black_wool",
        "glass",
        "glass_pane",
        "white_stained_glass",
        "white_stained_glass_pane",
        "cobweb",
        "vine",
        "glow_lichen",
    ] {
        m.insert(block, ToolType::Shears);
    }

    m
});

/// Speed multipliers for each material tier when mining with the correct tool.
pub static MATERIAL_TIER_SPEED: LazyLock<HashMap<MaterialTier, f64>> = LazyLock::new(|| {
    let mut m = HashMap::new();
    m.insert(MaterialTier::Wood, 2.0);
    m.insert(MaterialTier::Stone, 4.0);
    m.insert(MaterialTier::Iron, 6.0);
    m.insert(MaterialTier::Diamond, 8.0);
    m.insert(MaterialTier::Netherite, 9.0);
    m.insert(MaterialTier::Gold, 12.0);
    m
});

/// Maps `(ToolType, MaterialTier)` pairs to the Minecraft item names they
/// correspond to.
pub static TOOL_NAMES: LazyLock<HashMap<(ToolType, MaterialTier), Vec<&'static str>>> =
    LazyLock::new(|| {
        let mut m = HashMap::new();

        m.insert(
            (ToolType::Pickaxe, MaterialTier::Wood),
            vec!["wooden_pickaxe"],
        );
        m.insert(
            (ToolType::Pickaxe, MaterialTier::Stone),
            vec!["stone_pickaxe"],
        );
        m.insert(
            (ToolType::Pickaxe, MaterialTier::Iron),
            vec!["iron_pickaxe"],
        );
        m.insert(
            (ToolType::Pickaxe, MaterialTier::Gold),
            vec!["golden_pickaxe"],
        );
        m.insert(
            (ToolType::Pickaxe, MaterialTier::Diamond),
            vec!["diamond_pickaxe"],
        );
        m.insert(
            (ToolType::Pickaxe, MaterialTier::Netherite),
            vec!["netherite_pickaxe"],
        );

        m.insert((ToolType::Axe, MaterialTier::Wood), vec!["wooden_axe"]);
        m.insert((ToolType::Axe, MaterialTier::Stone), vec!["stone_axe"]);
        m.insert((ToolType::Axe, MaterialTier::Iron), vec!["iron_axe"]);
        m.insert((ToolType::Axe, MaterialTier::Gold), vec!["golden_axe"]);
        m.insert((ToolType::Axe, MaterialTier::Diamond), vec!["diamond_axe"]);
        m.insert(
            (ToolType::Axe, MaterialTier::Netherite),
            vec!["netherite_axe"],
        );

        m.insert(
            (ToolType::Shovel, MaterialTier::Wood),
            vec!["wooden_shovel"],
        );
        m.insert(
            (ToolType::Shovel, MaterialTier::Stone),
            vec!["stone_shovel"],
        );
        m.insert((ToolType::Shovel, MaterialTier::Iron), vec!["iron_shovel"]);
        m.insert(
            (ToolType::Shovel, MaterialTier::Gold),
            vec!["golden_shovel"],
        );
        m.insert(
            (ToolType::Shovel, MaterialTier::Diamond),
            vec!["diamond_shovel"],
        );
        m.insert(
            (ToolType::Shovel, MaterialTier::Netherite),
            vec!["netherite_shovel"],
        );

        m.insert((ToolType::Shears, MaterialTier::Iron), vec!["shears"]);

        m
    });

/// Hardness values for common Minecraft blocks.
///
/// Values are in seconds of mining time with fist (no tool).
/// A value of `-1.0` indicates an unbreakable block (bedrock).
pub static BLOCK_HARDNESS: LazyLock<HashMap<&'static str, f64>> = LazyLock::new(|| {
    let mut m = HashMap::new();

    // Stone variants
    m.insert("stone", 1.5);
    m.insert("cobblestone", 2.0);
    m.insert("andesite", 1.5);
    m.insert("diorite", 1.5);
    m.insert("granite", 1.5);
    m.insert("stone_bricks", 1.5);
    m.insert("deepslate", 3.0);
    m.insert("tuff", 1.5);
    m.insert("calcite", 0.75);
    m.insert("bedrock", -1.0);

    // Ores
    m.insert("coal_ore", 3.0);
    m.insert("iron_ore", 3.0);
    m.insert("gold_ore", 3.0);
    m.insert("diamond_ore", 3.0);
    m.insert("emerald_ore", 3.0);
    m.insert("lapis_ore", 3.0);
    m.insert("redstone_ore", 3.0);
    m.insert("copper_ore", 3.0);
    m.insert("deepslate_coal_ore", 4.5);
    m.insert("deepslate_iron_ore", 4.5);
    m.insert("deepslate_gold_ore", 4.5);
    m.insert("deepslate_diamond_ore", 4.5);
    m.insert("deepslate_emerald_ore", 4.5);
    m.insert("deepslate_lapis_ore", 4.5);
    m.insert("deepslate_redstone_ore", 4.5);
    m.insert("deepslate_copper_ore", 4.5);

    // Nether / End
    m.insert("netherrack", 0.4);
    m.insert("nether_quartz_ore", 3.0);
    m.insert("nether_gold_ore", 3.0);
    m.insert("end_stone", 3.0);

    // Wood
    m.insert("oak_log", 2.0);
    m.insert("spruce_log", 2.0);
    m.insert("birch_log", 2.0);
    m.insert("jungle_log", 2.0);
    m.insert("oak_planks", 2.0);
    m.insert("crafting_table", 2.5);
    m.insert("bookshelf", 1.5);

    // Dirt & sand
    m.insert("dirt", 0.5);
    m.insert("grass_block", 0.6);
    m.insert("sand", 0.5);
    m.insert("gravel", 0.6);
    m.insert("clay", 0.6);
    m.insert("soul_sand", 0.5);

    // Other
    m.insert("oak_leaves", 0.2);
    m.insert("white_wool", 0.8);
    m.insert("glass", 0.3);
    m.insert("ice", 0.5);
    m.insert("snow", 0.1);

    // Notable blocks
    m.insert("obsidian", 50.0);
    m.insert("furnace", 3.5);
    m.insert("anvil", 5.0);
    m.insert("enchanting_table", 5.0);
    m.insert("ender_chest", 22.5);
    m.insert("iron_block", 5.0);
    m.insert("diamond_block", 5.0);

    m
});

/// Material tier priority order — from best (index 0) to worst (index N).
///
/// Used by [`find_best_tool_in_inventory`] to select the highest-tier tool.
pub static MATERIAL_PRIORITY: &[MaterialTier] = &[
    MaterialTier::Netherite,
    MaterialTier::Diamond,
    MaterialTier::Iron,
    MaterialTier::Stone,
    MaterialTier::Wood,
    MaterialTier::Gold,
];

// ---------------------------------------------------------------------------
// Lookup functions
// ---------------------------------------------------------------------------

/// Returns the best [`ToolType`] for mining the given block.
///
/// Returns [`ToolType::Hand`] for unknown blocks.
pub fn best_tool_for_block(block_type: &str) -> ToolType {
    BLOCK_TO_TOOL_TYPE
        .get(block_type)
        .copied()
        .unwrap_or(ToolType::Hand)
}

/// Parses a Minecraft item name into its `(ToolType, MaterialTier)`.
///
/// Supports names like `"iron_pickaxe"`, `"diamond_axe"`, `"shears"`, etc.
/// Returns `None` for non-tool items or unrecognised names.
pub fn material_from_item_name(name: &str) -> Option<(ToolType, MaterialTier)> {
    let parts: Vec<&str> = name.split('_').collect();

    match parts.len() {
        1 => match name {
            "shears" => Some((ToolType::Shears, MaterialTier::Iron)),
            _ => None,
        },
        2 => {
            let material = match parts[0] {
                "wooden" => Some(MaterialTier::Wood),
                "stone" => Some(MaterialTier::Stone),
                "iron" => Some(MaterialTier::Iron),
                "golden" => Some(MaterialTier::Gold),
                "diamond" => Some(MaterialTier::Diamond),
                "netherite" => Some(MaterialTier::Netherite),
                _ => None,
            };
            let tool = match parts[1] {
                "pickaxe" => Some(ToolType::Pickaxe),
                "axe" => Some(ToolType::Axe),
                "shovel" => Some(ToolType::Shovel),
                _ => None,
            };
            match (material, tool) {
                (Some(m), Some(t)) => Some((t, m)),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Calculates the time (in seconds) to mine a block with the given tool and
/// material.
///
/// If the tool is the wrong type for the block, a 5× penalty is applied.
/// Unbreakable blocks (hardness < 0) return [`f64::INFINITY`].
/// Unknown blocks default to 0.5 hardness.
pub fn calculate_mine_time(block_type: &str, tool_type: &ToolType, material: &MaterialTier) -> f64 {
    let hardness = BLOCK_HARDNESS.get(block_type).copied().unwrap_or(0.5);

    // Unbreakable blocks (bedrock, etc.)
    if hardness < 0.0 {
        return f64::INFINITY;
    }

    let speed = if *tool_type == ToolType::Hand {
        1.0
    } else {
        MATERIAL_TIER_SPEED.get(material).copied().unwrap_or(1.0)
    };

    let expected_tool = best_tool_for_block(block_type);
    let penalty = if *tool_type != ToolType::Hand && *tool_type != expected_tool {
        5.0
    } else {
        1.0
    };

    hardness / speed * penalty
}

/// Finds the best available tool of the given type in an inventory.
///
/// Returns the slot index of the best tool (highest material priority), or
/// `None` if no matching tool is found.
pub fn find_best_tool_in_inventory(
    tool_type: &ToolType,
    inventory: &[Option<ItemStack>],
) -> Option<u8> {
    let mut best_slot: Option<u8> = None;
    let mut best_priority: Option<usize> = None;

    for (slot, stack) in inventory.iter().enumerate() {
        let stack = match stack {
            Some(s) => s,
            None => continue,
        };

        if let Some((found_tool, found_material)) = material_from_item_name(&stack.item_id) {
            if &found_tool != tool_type {
                continue;
            }

            let priority = MATERIAL_PRIORITY.iter().position(|m| m == &found_material);

            match (best_priority, priority) {
                (None, Some(p)) => {
                    best_slot = Some(slot as u8);
                    best_priority = Some(p);
                }
                (Some(best), Some(p)) if p < best => {
                    // Lower index = higher priority
                    best_slot = Some(slot as u8);
                    best_priority = Some(p);
                }
                _ => {}
            }
        }
    }

    best_slot
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{MaterialTier, ToolType};

    // --- best_tool_for_block ---

    #[test]
    fn test_best_tool_for_block_stone() {
        assert_eq!(best_tool_for_block("stone"), ToolType::Pickaxe);
        assert_eq!(best_tool_for_block("cobblestone"), ToolType::Pickaxe);
        assert_eq!(best_tool_for_block("iron_ore"), ToolType::Pickaxe);
        assert_eq!(best_tool_for_block("deepslate"), ToolType::Pickaxe);
    }

    #[test]
    fn test_best_tool_for_block_wood() {
        assert_eq!(best_tool_for_block("oak_log"), ToolType::Axe);
        assert_eq!(best_tool_for_block("oak_planks"), ToolType::Axe);
        assert_eq!(best_tool_for_block("crafting_table"), ToolType::Axe);
    }

    #[test]
    fn test_best_tool_for_block_dirt() {
        assert_eq!(best_tool_for_block("dirt"), ToolType::Shovel);
        assert_eq!(best_tool_for_block("sand"), ToolType::Shovel);
        assert_eq!(best_tool_for_block("gravel"), ToolType::Shovel);
    }

    #[test]
    fn test_best_tool_for_block_special() {
        assert_eq!(best_tool_for_block("white_wool"), ToolType::Shears);
        assert_eq!(best_tool_for_block("oak_leaves"), ToolType::Shears);
        assert_eq!(best_tool_for_block("glass"), ToolType::Shears);
    }

    #[test]
    fn test_best_tool_for_block_unknown() {
        assert_eq!(best_tool_for_block("unknown_block"), ToolType::Hand);
        assert_eq!(best_tool_for_block("not_a_block"), ToolType::Hand);
    }

    // --- MATERIAL_TIER_SPEED ---

    #[test]
    fn test_material_tier_speed_values() {
        assert_eq!(
            MATERIAL_TIER_SPEED.get(&MaterialTier::Wood).copied(),
            Some(2.0)
        );
        assert_eq!(
            MATERIAL_TIER_SPEED.get(&MaterialTier::Stone).copied(),
            Some(4.0)
        );
        assert_eq!(
            MATERIAL_TIER_SPEED.get(&MaterialTier::Iron).copied(),
            Some(6.0)
        );
        assert_eq!(
            MATERIAL_TIER_SPEED.get(&MaterialTier::Diamond).copied(),
            Some(8.0)
        );
        assert_eq!(
            MATERIAL_TIER_SPEED.get(&MaterialTier::Netherite).copied(),
            Some(9.0)
        );
        assert_eq!(
            MATERIAL_TIER_SPEED.get(&MaterialTier::Gold).copied(),
            Some(12.0)
        );
    }

    // --- material_from_item_name ---

    #[test]
    fn test_material_from_item_pickaxe() {
        assert_eq!(
            material_from_item_name("iron_pickaxe"),
            Some((ToolType::Pickaxe, MaterialTier::Iron))
        );
        assert_eq!(
            material_from_item_name("diamond_pickaxe"),
            Some((ToolType::Pickaxe, MaterialTier::Diamond))
        );
        assert_eq!(
            material_from_item_name("wooden_pickaxe"),
            Some((ToolType::Pickaxe, MaterialTier::Wood))
        );
    }

    #[test]
    fn test_material_from_item_axe() {
        assert_eq!(
            material_from_item_name("diamond_axe"),
            Some((ToolType::Axe, MaterialTier::Diamond))
        );
        assert_eq!(
            material_from_item_name("netherite_axe"),
            Some((ToolType::Axe, MaterialTier::Netherite))
        );
    }

    #[test]
    fn test_material_from_item_shovel() {
        assert_eq!(
            material_from_item_name("iron_shovel"),
            Some((ToolType::Shovel, MaterialTier::Iron))
        );
    }

    #[test]
    fn test_material_from_item_shears() {
        assert_eq!(
            material_from_item_name("shears"),
            Some((ToolType::Shears, MaterialTier::Iron))
        );
    }

    #[test]
    fn test_material_from_item_unknown() {
        assert_eq!(material_from_item_name("unknown_item"), None);
        assert_eq!(material_from_item_name("diamond_sword"), None);
        assert_eq!(material_from_item_name("stone"), None);
    }

    // --- calculate_mine_time ---

    #[test]
    fn test_mine_time_stone_with_iron_pickaxe() {
        // stone hardness = 1.5, iron speed = 6.0, correct tool
        let time = calculate_mine_time("stone", &ToolType::Pickaxe, &MaterialTier::Iron);
        assert!((time - 0.25).abs() < f64::EPSILON); // 1.5 / 6.0 = 0.25
    }

    #[test]
    fn test_mine_time_stone_with_iron_axe_wrong_tool() {
        // stone hardness = 1.5, iron speed = 6.0, wrong tool = 5×
        let time = calculate_mine_time("stone", &ToolType::Axe, &MaterialTier::Iron);
        assert!((time - 1.25).abs() < f64::EPSILON); // 1.5 / 6.0 × 5 = 1.25
    }

    #[test]
    fn test_mine_time_obsidian_with_diamond() {
        // obsidian = 50.0, diamond speed = 8.0
        let time = calculate_mine_time("obsidian", &ToolType::Pickaxe, &MaterialTier::Diamond);
        assert!((time - 6.25).abs() < f64::EPSILON); // 50.0 / 8.0 = 6.25
    }

    #[test]
    fn test_mine_time_unknown_block_hand() {
        // default 0.5 / 1.0 = 0.5 (hand has no penalty applied)
        let time = calculate_mine_time("unknown_block", &ToolType::Hand, &MaterialTier::Wood);
        assert!((time - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_mine_time_unbreakable() {
        let time = calculate_mine_time("bedrock", &ToolType::Pickaxe, &MaterialTier::Netherite);
        assert_eq!(time, f64::INFINITY);
    }

    #[test]
    fn test_mine_time_gold_max_speed() {
        // dirt = 0.5, gold speed = 12.0
        let time = calculate_mine_time("dirt", &ToolType::Shovel, &MaterialTier::Gold);
        assert!((time - 0.5 / 12.0).abs() < f64::EPSILON);
    }

    // --- find_best_tool_in_inventory ---

    #[test]
    fn test_find_best_tool_empty_inventory() {
        let inv: Vec<Option<ItemStack>> = vec![];
        assert_eq!(find_best_tool_in_inventory(&ToolType::Pickaxe, &inv), None);
    }

    #[test]
    fn test_find_best_tool_none_match() {
        let inv = vec![
            Some(ItemStack {
                item_id: "dirt".to_string(),
                count: 1,
            }),
            Some(ItemStack {
                item_id: "stone".to_string(),
                count: 1,
            }),
        ];
        assert_eq!(find_best_tool_in_inventory(&ToolType::Pickaxe, &inv), None);
    }

    #[test]
    fn test_find_best_tool_selects_higher_priority() {
        let inv = vec![
            None,
            Some(ItemStack {
                item_id: "wooden_pickaxe".to_string(),
                count: 1,
            }),
            None,
            Some(ItemStack {
                item_id: "iron_pickaxe".to_string(),
                count: 1,
            }),
            Some(ItemStack {
                item_id: "stone_axe".to_string(),
                count: 1,
            }),
        ];
        // slot 3 = iron_pickaxe (higher priority than wooden at slot 1)
        assert_eq!(
            find_best_tool_in_inventory(&ToolType::Pickaxe, &inv),
            Some(3)
        );
    }

    #[test]
    fn test_find_best_tool_diamond_priority() {
        let inv = vec![
            Some(ItemStack {
                item_id: "iron_shovel".to_string(),
                count: 1,
            }),
            Some(ItemStack {
                item_id: "diamond_shovel".to_string(),
                count: 1,
            }),
            Some(ItemStack {
                item_id: "golden_shovel".to_string(),
                count: 1,
            }),
        ];
        // Diamond (index 1) > Iron (index 2) > Gold (index 5)
        // So slot 1 (diamond) is best
        assert_eq!(
            find_best_tool_in_inventory(&ToolType::Shovel, &inv),
            Some(1)
        );
    }

    #[test]
    fn test_find_best_tool_netherite_best() {
        let inv = vec![
            Some(ItemStack {
                item_id: "netherite_pickaxe".to_string(),
                count: 1,
            }),
            Some(ItemStack {
                item_id: "diamond_pickaxe".to_string(),
                count: 1,
            }),
        ];
        // Netherite (index 0) > Diamond (index 1)
        assert_eq!(
            find_best_tool_in_inventory(&ToolType::Pickaxe, &inv),
            Some(0)
        );
    }

    // --- MATERIAL_PRIORITY order ---

    #[test]
    fn test_material_priority_order() {
        assert_eq!(
            MATERIAL_PRIORITY
                .iter()
                .position(|m| matches!(m, MaterialTier::Netherite)),
            Some(0)
        );
        assert_eq!(
            MATERIAL_PRIORITY
                .iter()
                .position(|m| matches!(m, MaterialTier::Diamond)),
            Some(1)
        );
        assert_eq!(
            MATERIAL_PRIORITY
                .iter()
                .position(|m| matches!(m, MaterialTier::Iron)),
            Some(2)
        );
        assert_eq!(
            MATERIAL_PRIORITY
                .iter()
                .position(|m| matches!(m, MaterialTier::Stone)),
            Some(3)
        );
        assert_eq!(
            MATERIAL_PRIORITY
                .iter()
                .position(|m| matches!(m, MaterialTier::Wood)),
            Some(4)
        );
        assert_eq!(
            MATERIAL_PRIORITY
                .iter()
                .position(|m| matches!(m, MaterialTier::Gold)),
            Some(5)
        );
    }

    // --- TOOL_NAMES symmetry ---

    #[test]
    fn test_tool_names_roundtrip() {
        for ((tool_type, mat_tier), names) in TOOL_NAMES.iter() {
            for name in names {
                let parsed = material_from_item_name(name);
                assert_eq!(
                    parsed,
                    Some((*tool_type, *mat_tier)),
                    "TOOL_NAMES entry '{name}' failed roundtrip"
                );
            }
        }
    }
}
