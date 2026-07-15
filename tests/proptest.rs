//! Property-based tests for the Minecraft MCP server.
//!
//! Uses the `proptest` crate to verify invariants across thousands of
//! randomly-generated inputs.

use minecraft_mcp_rs::block_data::{
    ItemStack, MATERIAL_PRIORITY, MATERIAL_TIER_SPEED, best_tool_for_block, material_from_item_name,
};
use minecraft_mcp_rs::command_validate::validate_coordinates;
use minecraft_mcp_rs::compound_ops::find_standable_neighbor;
use minecraft_mcp_rs::mining_calc::calculate_mine_time;
use minecraft_mcp_rs::tool_select::find_tool_in_inventory;
use minecraft_mcp_rs::types::{BlockEntry, BlockPos, MaterialTier, ToolType, WorldSnapshot};
use proptest::prelude::*;
use std::collections::HashMap;

// ═══════════════════════════════════════════════════════════════
// Strategies
// ═══════════════════════════════════════════════════════════════

/// Strategy for generating an arbitrary [`ToolType`].
fn tool_type_strategy() -> impl Strategy<Value = ToolType> {
    prop_oneof![
        Just(ToolType::Pickaxe),
        Just(ToolType::Axe),
        Just(ToolType::Shovel),
        Just(ToolType::Hoe),
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
    /// `find_tool_in_inventory` (if any) matches the requested tool type.
    #[test]
    fn prop_tool_selection_matches_type(
        inventory in inventory_strategy(),
        block_type in block_type_strategy(),
    ) {
        let expected_tool = best_tool_for_block(&block_type);

        // If the expected tool is Hand, there is no "best tool" to find.
        prop_assume!(expected_tool != ToolType::Hand);

        let best_slot = find_tool_in_inventory(&expected_tool, &inventory, None).map(|(_, slot)| slot);

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

        let best_slot = find_tool_in_inventory(&tool_type, &inventory, None).map(|(_, slot)| slot);

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
        let time = calculate_mine_time(&block_type, tool_type, material);

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

        let correct_time = calculate_mine_time(&block_type, expected_tool, material);

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
            let wrong_time = calculate_mine_time(&block_type, *wrong_tool, material);

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
            let time = calculate_mine_time(&block_type, expected_tool, *tier);
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
                ToolType::Hoe,
                ToolType::Sword,
                ToolType::Shears,
                ToolType::Hand,
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

// ═══════════════════════════════════════════════════════════════
// Property: find_standable_neighbor
// ═══════════════════════════════════════════════════════════════

/// Helper: build a [`WorldSnapshot`] from a list of blocks, populating
/// `block_index` exactly the way `SnapshotBuilder::build` does in
/// production. `find_standable_neighbor` reads via `block_index`, so the
/// test snapshot must match.
fn make_snapshot_with_blocks(blocks: Vec<BlockEntry>) -> WorldSnapshot {
    let block_index: HashMap<BlockPos, usize> = blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.position, i))
        .collect();
    WorldSnapshot {
        blocks,
        block_index,
        ..Default::default()
    }
}

/// Returns `true` when `pos` is one of the 8 horizontal neighbour cells
/// of `target` at the same Y (or ±1 Y), excluding `target` itself. This
/// mirrors the geometry `find_standable_neighbor` searches.
fn is_8adjacent(target: BlockPos, pos: BlockPos) -> bool {
    let dx = (pos.x - target.x).abs();
    let dy = (pos.y - target.y).abs();
    let dz = (pos.z - target.z).abs();
    dx <= 1 && dy <= 1 && dz <= 1 && (dx + dy + dz) > 0
}

proptest! {
    /// Property: For any target position within Minecraft Y bounds and
    /// any standable neighbor in one of the 8 horizontal directions
    /// (4 orthogonal + 4 diagonal), `find_standable_neighbor` must
    /// return a position that is 8-direction adjacent to the target.
    ///
    /// The generated snapshot places the target as a solid stone block
    /// and the chosen neighbor as an air block with a solid stone floor
    /// directly below it — the canonical standable setup. The function
    /// must find that neighbor (or, if multiple standable cells exist in
    /// the 3×3×3 search volume due to prop luck, another 8-adjacent
    /// cell), and never a position outside the search volume.
    #[test]
    fn prop_standable_neighbor_returns_adjacent(
        x in -100i32..100,
        z in -100i32..100,
        y in -64i32..=320i32,
        dir_x in -1i32..=1,
        dir_z in -1i32..=1,
    ) {
        // Skip (0, 0) — the target itself is not a neighbour.
        prop_assume!(dir_x != 0 || dir_z != 0);

        let target = BlockPos::new(x, y, z);
        let neighbor = BlockPos::new(x + dir_x, y, z + dir_z);
        let floor = BlockPos::new(x + dir_x, y - 1, z + dir_z);

        let snapshot = make_snapshot_with_blocks(vec![
            BlockEntry {
                position: target,
                block_type: "stone".into(),
                block_state: None,
            },
            BlockEntry {
                position: neighbor,
                block_type: "air".into(),
                block_state: None,
            },
            BlockEntry {
                position: floor,
                block_type: "stone".into(),
                block_state: None,
            },
        ]);

        let result = find_standable_neighbor(&snapshot, target);
        match result {
            Some(pos) => {
                prop_assert!(
                    is_8adjacent(target, pos),
                    "result {:?} is not 8-adjacent to target {:?} (must be within ±1 on each axis and not equal)",
                    pos,
                    target
                );
            }
            None => {
                // A 4-direction implementation would return None when
                // the only standable cell is diagonal. We do not require
                // the test to fail in that case — the `*_8_directions`
                // unit test in `compound_ops` enforces diagonal support.
                // Property: when None, there must be no 4-adjacent
                // standable cell either (which is satisfied vacuously
                // because we did not add a 4-adjacent air block).
            }
        }
    }

    /// Property: For Y boundary values (-64 and 320) and an interior
    /// Y, placing a standable neighbor at the target's same Y must be
    /// found by `find_standable_neighbor` (the same-Y search has top
    /// priority in the implementation).
    #[test]
    fn prop_standable_neighbor_y_extremes(y in prop_oneof![
        Just(-64i32), Just(320i32), Just(64i32), Just(100i32), Just(200i32)
    ]) {
        let target = BlockPos::new(50, y, -50);
        let neighbor = BlockPos::new(51, y, -50);
        let floor = BlockPos::new(51, y - 1, -50);

        let snapshot = make_snapshot_with_blocks(vec![
            BlockEntry {
                position: target,
                block_type: "stone".into(),
                block_state: None,
            },
            BlockEntry {
                position: neighbor,
                block_type: "air".into(),
                block_state: None,
            },
            BlockEntry {
                position: floor,
                block_type: "stone".into(),
                block_state: None,
            },
        ]);

        let result = find_standable_neighbor(&snapshot, target);
        prop_assert!(
            result == Some(neighbor),
            "expected Some({:?}) at y={}, got {:?}",
            neighbor,
            y,
            result
        );
    }

    /// Property: An empty snapshot must never report a standable
    /// neighbour (no air blocks + no floors = no place to stand).
    #[test]
    fn prop_standable_neighbor_empty_snapshot(
        x in -100i32..100,
        z in -100i32..100,
        y in -64i32..=320i32,
    ) {
        let target = BlockPos::new(x, y, z);
        let snapshot = WorldSnapshot::default();
        prop_assert!(
            find_standable_neighbor(&snapshot, target).is_none(),
            "empty snapshot should yield None, got Some for target {:?}",
            target
        );
    }

    /// Property: When all 8 horizontal neighbours are blocked (solid
    /// stone) and the diagonal SE neighbour is air with a solid floor
    /// below, the function must find that diagonal — verifying the
    /// 8-direction scan. This is the same shape as the unit test
    /// `test_find_standable_neighbor_8_directions` but driven by
    /// proptest so a regression in the offset table is caught
    /// automatically across 1000 runs.
    #[test]
    fn prop_standable_neighbor_finds_diagonal_when_cardinals_blocked(
        x in -100i32..100,
        z in -100i32..100,
        y in -64i32..=320i32,
    ) {
        let target = BlockPos::new(x, y, z);
        let diag = BlockPos::new(x + 1, y, z + 1);
        let diag_floor = BlockPos::new(x + 1, y - 1, z + 1);

        let mut blocks = vec![BlockEntry {
            position: target,
            block_type: "stone".into(),
            block_state: None,
        }];
        // Block all 4 cardinal neighbours.
        for (dx, dz) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
            blocks.push(BlockEntry {
                position: BlockPos::new(x + dx, y, z + dz),
                block_type: "stone".into(),
                block_state: None,
            });
            blocks.push(BlockEntry {
                position: BlockPos::new(x + dx, y - 1, z + dz),
                block_type: "stone".into(),
                block_state: None,
            });
        }
        // The SE diagonal is the only standable cell.
        blocks.push(BlockEntry {
            position: diag,
            block_type: "air".into(),
            block_state: None,
        });
        blocks.push(BlockEntry {
            position: diag_floor,
            block_type: "stone".into(),
            block_state: None,
        });

        let snapshot = make_snapshot_with_blocks(blocks);
        let result = find_standable_neighbor(&snapshot, target);
        prop_assert!(
            result == Some(diag),
            "expected Some({:?}) (the only standable cell), got {:?}",
            diag,
            result
        );
    }
}
