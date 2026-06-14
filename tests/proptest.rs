//! Property-based tests for the Minecraft MCP server.
//!
//! Uses the `proptest` crate to verify invariants across thousands of
//! randomly-generated inputs.

use minecraft_mcp_rs::block_data::{
    ItemStack, MATERIAL_PRIORITY, MATERIAL_TIER_SPEED, best_tool_for_block, calculate_mine_time,
    find_best_tool_in_inventory, material_from_item_name,
};
use minecraft_mcp_rs::command_validate::validate_coordinates;
use minecraft_mcp_rs::types::{MaterialTier, ToolType};
use proptest::prelude::*;

// ═══════════════════════════════════════════════════════════════
// Strategies
// ═══════════════════════════════════════════════════════════════

/// Strategy for generating an arbitrary [`ToolType`].
fn tool_type_strategy() -> impl Strategy<Value = ToolType> {
    prop_oneof![
        Just(ToolType::Pickaxe),
        Just(ToolType::Axe),
        Just(ToolType::Shovel),
        Just(ToolType::Sword),
        Just(ToolType::Shears),
        Just(ToolType::Hand),
    ]
}

/// Strategy for generating an arbitrary [`MaterialTier`].
fn material_tier_strategy() -> impl Strategy<Value = MaterialTier> {
    prop_oneof![
        Just(MaterialTier::Wood),
        Just(MaterialTier::Stone),
        Just(MaterialTier::Iron),
        Just(MaterialTier::Gold),
        Just(MaterialTier::Diamond),
        Just(MaterialTier::Netherite),
    ]
}

/// Strategy for generating a valid Minecraft item name.
///
/// Produces either known tool names (e.g. `iron_pickaxe`) or random
/// alphanumeric strings that may or may not parse as tools.
fn item_name_strategy() -> impl Strategy<Value = String> {
    let known_tools = prop_oneof![
        Just("wooden_pickaxe".to_string()),
        Just("stone_pickaxe".to_string()),
        Just("iron_pickaxe".to_string()),
        Just("golden_pickaxe".to_string()),
        Just("diamond_pickaxe".to_string()),
        Just("netherite_pickaxe".to_string()),
        Just("wooden_axe".to_string()),
        Just("stone_axe".to_string()),
        Just("iron_axe".to_string()),
        Just("golden_axe".to_string()),
        Just("diamond_axe".to_string()),
        Just("netherite_axe".to_string()),
        Just("wooden_shovel".to_string()),
        Just("stone_shovel".to_string()),
        Just("iron_shovel".to_string()),
        Just("golden_shovel".to_string()),
        Just("diamond_shovel".to_string()),
        Just("netherite_shovel".to_string()),
        Just("shears".to_string()),
        Just("dirt".to_string()),
        Just("stone".to_string()),
        Just("diamond_sword".to_string()),
    ];

    let random_name = "[a-z_]{1,20}".prop_map(|s| s);

    prop_oneof![7 => known_tools, 3 => random_name]
}

/// Strategy for generating an arbitrary inventory slot.
fn inventory_slot_strategy() -> impl Strategy<Value = Option<ItemStack>> {
    prop_oneof![
        4 => Just(None),
        6 => item_name_strategy().prop_map(|name| Some(ItemStack {
            item_id: name,
            count: 1,
        })),
    ]
}

/// Strategy for generating an arbitrary inventory (0-36 slots).
fn inventory_strategy() -> impl Strategy<Value = Vec<Option<ItemStack>>> {
    prop::collection::vec(inventory_slot_strategy(), 0..=36)
}

/// Strategy for generating a known or unknown block type.
fn block_type_strategy() -> impl Strategy<Value = String> {
    let known_blocks = prop_oneof![
        Just("stone".to_string()),
        Just("cobblestone".to_string()),
        Just("dirt".to_string()),
        Just("grass_block".to_string()),
        Just("oak_log".to_string()),
        Just("oak_planks".to_string()),
        Just("bedrock".to_string()),
        Just("obsidian".to_string()),
        Just("iron_ore".to_string()),
        Just("diamond_ore".to_string()),
        Just("sand".to_string()),
        Just("gravel".to_string()),
        Just("white_wool".to_string()),
        Just("glass".to_string()),
        Just("netherrack".to_string()),
        Just("end_stone".to_string()),
        Just("deepslate".to_string()),
        Just("furnace".to_string()),
        Just("anvil".to_string()),
        Just("ender_chest".to_string()),
    ];

    let random_block = "[a-z_]{1,20}".prop_map(|s| s);

    prop_oneof![7 => known_blocks, 3 => random_block]
}

// ═══════════════════════════════════════════════════════════════
// Property: Tool Selection
// ═══════════════════════════════════════════════════════════════

proptest! {
    /// Property: For any inventory and any block type, the tool selected by
    /// `find_best_tool_in_inventory` (if any) matches the requested tool type.
    #[test]
    fn prop_tool_selection_matches_type(
        inventory in inventory_strategy(),
        block_type in block_type_strategy(),
    ) {
        let expected_tool = best_tool_for_block(&block_type);

        // If the expected tool is Hand, there is no "best tool" to find.
        prop_assume!(expected_tool != ToolType::Hand);

        let best_slot = find_best_tool_in_inventory(&expected_tool, &inventory);

        if let Some(slot) = best_slot {
            let slot = slot as usize;
            prop_assert!(
                slot < inventory.len(),
                "Selected slot {slot} is out of bounds (inventory len = {})",
                inventory.len()
            );

            let stack = inventory[slot].as_ref().unwrap();
            let parsed = material_from_item_name(&stack.item_id);
            prop_assert!(
                parsed.is_some(),
                "Selected item '{}' does not parse as a tool",
                stack.item_id
            );

            let (found_tool, _found_material) = parsed.unwrap();
            prop_assert_eq!(
                found_tool, expected_tool,
                "Selected tool type {:?} does not match expected {:?}",
                found_tool, expected_tool
            );
        }
    }

    /// Property: When a tool is found, its material tier is the highest
    /// available among all matching tools in the inventory.
    #[test]
    fn prop_tool_selection_highest_tier(
        inventory in inventory_strategy(),
        tool_type in tool_type_strategy(),
    ) {
        // Skip Hand since there's no tier comparison for it
        prop_assume!(tool_type != ToolType::Hand);

        let best_slot = find_best_tool_in_inventory(&tool_type, &inventory);

        if let Some(best_slot) = best_slot {
            let best_stack = inventory[best_slot as usize].as_ref().unwrap();
            let (_, best_material) = material_from_item_name(&best_stack.item_id).unwrap();
            let best_priority = MATERIAL_PRIORITY
                .iter()
                .position(|m| m == &best_material)
                .unwrap();

            for (slot, stack) in inventory.iter().enumerate() {
                let stack = match stack {
                    Some(s) => s,
                    None => continue,
                };

                if let Some((found_tool, found_material)) = material_from_item_name(&stack.item_id)
                    && found_tool == tool_type
                {
                    let found_priority = MATERIAL_PRIORITY
                        .iter()
                        .position(|m| m == &found_material)
                        .unwrap();
                    prop_assert!(
                        found_priority >= best_priority,
                        "Slot {slot} has tool with better priority ({found_priority}) \
                         than selected slot {best_slot} ({best_priority})"
                    );
                }
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// Property: Mining Time Calculation
// ═══════════════════════════════════════════════════════════════

proptest! {
    /// Property: Mining time is always positive or INFINITY.
    #[test]
    fn prop_mining_time_positive_or_infinite(
        block_type in block_type_strategy(),
        tool_type in tool_type_strategy(),
        material in material_tier_strategy(),
    ) {
        let time = calculate_mine_time(&block_type, &tool_type, &material);

        prop_assert!(
            time > 0.0 || time.is_infinite(),
            "Mining time must be positive or INFINITY, got {time} \
             for block '{block_type}' with tool {:?} material {:?}",
            tool_type, material
        );
    }

    /// Property: For known blocks, the correct tool mines faster than
    /// the wrong tool (same material tier).
    #[test]
    fn prop_correct_tool_faster_than_wrong_tool(
        block_type in block_type_strategy(),
        material in material_tier_strategy(),
    ) {
        let expected_tool = best_tool_for_block(&block_type);

        // Skip Hand blocks (no "wrong" tool penalty applies to Hand)
        // and unbreakable blocks (time is INFINITY regardless).
        prop_assume!(expected_tool != ToolType::Hand);

        let correct_time = calculate_mine_time(&block_type, &expected_tool, &material);

        // If unbreakable, both should be INFINITY
        prop_assume!(!correct_time.is_infinite());

        // Pick a wrong tool type (different from expected)
        let wrong_tools: Vec<ToolType> = [
            ToolType::Pickaxe,
            ToolType::Axe,
            ToolType::Shovel,
            ToolType::Sword,
            ToolType::Shears,
        ]
        .into_iter()
        .filter(|t| *t != expected_tool && *t != ToolType::Hand)
        .collect();

        prop_assume!(!wrong_tools.is_empty());

        for wrong_tool in &wrong_tools {
            let wrong_time = calculate_mine_time(&block_type, wrong_tool, &material);

            // Wrong tool gets a 5x penalty (unless it's Hand, which has no penalty)
            if *wrong_tool != ToolType::Hand {
                prop_assert!(
                    wrong_time >= correct_time,
                    "Wrong tool {:?} ({wrong_time}) should be >= correct tool {:?} ({correct_time}) \
                     for block '{block_type}'",
                    wrong_tool, expected_tool
                );
            }
        }
    }

    /// Property: Higher-tier materials mine faster (or equal) than lower-tier
    /// materials for the same block and correct tool.
    #[test]
    fn prop_higher_tier_faster_or_equal(
        block_type in block_type_strategy(),
    ) {
        let expected_tool = best_tool_for_block(&block_type);
        prop_assume!(expected_tool != ToolType::Hand);

        let tiers = [
            MaterialTier::Wood,
            MaterialTier::Stone,
            MaterialTier::Iron,
            MaterialTier::Diamond,
            MaterialTier::Netherite,
            MaterialTier::Gold,
        ];

        let mut times = Vec::new();
        for tier in &tiers {
            let time = calculate_mine_time(&block_type, &expected_tool, tier);
            times.push((*tier, time));
        }

        // For each pair, if tier A has higher speed than tier B, its time should be <=.
        for i in 0..times.len() {
            for j in (i + 1)..times.len() {
                let (tier_a, time_a) = times[i];
                let (tier_b, time_b) = times[j];

                let speed_a = MATERIAL_TIER_SPEED.get(&tier_a).copied().unwrap_or(1.0);
                let speed_b = MATERIAL_TIER_SPEED.get(&tier_b).copied().unwrap_or(1.0);

                if speed_a > speed_b {
                    prop_assert!(
                        time_a <= time_b || time_a.is_infinite() || time_b.is_infinite(),
                        "Tier {:?} (speed {speed_a}, time {time_a}) should mine faster \
                         than tier {:?} (speed {speed_b}, time {time_b})",
                        tier_a, tier_b
                    );
                }
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// Property: Command Validation
// ═══════════════════════════════════════════════════════════════

proptest! {
    /// Property: `validate_coordinates` never panics for any i32 inputs.
    #[test]
    fn prop_validate_coordinates_no_panic(x: i32, y: i32, z: i32) {
        // This test simply verifies the function does not panic.
        let _ = validate_coordinates(x, y, z);
    }

    /// Property: Coordinates within Minecraft bounds always pass validation.
    #[test]
    fn prop_valid_coordinates_always_pass(
        x in -30_000_000i32..=30_000_000i32,
        y in -64i32..=320i32,
        z in -30_000_000i32..=30_000_000i32,
    ) {
        let result = validate_coordinates(x, y, z);
        prop_assert!(
            result.is_ok(),
            "Coordinates ({x}, {y}, {z}) should be valid but got error: {:?}",
            result.err()
        );
    }

    /// Property: Coordinates outside Minecraft bounds always fail validation.
    #[test]
    fn prop_out_of_range_coordinates_always_fail(
        x in prop_oneof![
            i32::MIN..-30_000_001i32,
            30_000_001i32..=i32::MAX,
        ],
        y in prop_oneof![
            i32::MIN..-65i32,
            321i32..=i32::MAX,
        ],
        z in prop_oneof![
            i32::MIN..-30_000_001i32,
            30_000_001i32..=i32::MAX,
        ],
    ) {
        let result = validate_coordinates(x, y, z);
        prop_assert!(
            result.is_err(),
            "Coordinates ({x}, {y}, {z}) should be invalid but passed validation"
        );
    }
}

// ═══════════════════════════════════════════════════════════════
// Property: material_from_item_name roundtrip
// ═══════════════════════════════════════════════════════════════

proptest! {
    /// Property: For any item name, `material_from_item_name` returns
    /// `None` or a valid `(ToolType, MaterialTier)` pair.
    #[test]
    fn prop_material_from_item_name_valid(
        name in "[a-z_]{1,30}",
    ) {
        let result = material_from_item_name(&name);

        if let Some((tool, material)) = result {
            // Verify the returned values are valid enum variants
            let valid_tools = [
                ToolType::Pickaxe,
                ToolType::Axe,
                ToolType::Shovel,
                ToolType::Shears,
            ];
            let valid_materials = [
                MaterialTier::Wood,
                MaterialTier::Stone,
                MaterialTier::Iron,
                MaterialTier::Gold,
                MaterialTier::Diamond,
                MaterialTier::Netherite,
            ];

            prop_assert!(
                valid_tools.contains(&tool),
                "Parsed tool {:?} is not a valid tool type",
                tool
            );
            prop_assert!(
                valid_materials.contains(&material),
                "Parsed material {:?} is not a valid material tier",
                material
            );
        }
    }
}
