//! Tool / inventory selection logic for mining and combat.

use crate::block_data::{ItemStack, BLOCK_TO_TOOL_TYPE, MATERIAL_PRIORITY};
use crate::types::{MaterialTier, ToolType};

/// The result of selecting a tool for a specific block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSelection {
    pub tool_type: ToolType,
    pub material: Option<MaterialTier>,
    pub hotbar_slot: Option<u8>,
    pub needs_move_to_hotbar: bool,
}

impl ToolSelection {
    /// Create a `Hand` selection when no tool is available.
    pub fn hand() -> Self {
        Self {
            tool_type: ToolType::Hand,
            material: None,
            hotbar_slot: None,
            needs_move_to_hotbar: false,
        }
    }
}

/// Re-export of [`crate::block_data::material_from_item_name`].
pub use crate::block_data::material_from_item_name;

/// Finds the best tool of the given type anywhere in the inventory.
///
/// Returns `(material_tier, slot_index)` for the highest-tier match,
/// or `None` if no matching tool is found.
pub fn find_tool_in_inventory(
    tool_type: &ToolType,
    inventory: &[Option<ItemStack>],
) -> Option<(MaterialTier, u8)> {
    let mut best: Option<(MaterialTier, u8)> = None;
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

            let priority = MATERIAL_PRIORITY
                .iter()
                .position(|m| m == &found_material);

            match (best_priority, priority) {
                (None, Some(p)) => {
                    best = Some((found_material, slot as u8));
                    best_priority = Some(p);
                }
                (Some(best_p), Some(p)) if p < best_p => {
                    best = Some((found_material, slot as u8));
                    best_priority = Some(p);
                }
                _ => {}
            }
        }
    }

    best
}

/// Selects the best tool for mining the given block type.
///
/// 1. Determines required tool type from [`BLOCK_TO_TOOL_TYPE`].
/// 2. Searches hotbar (slots 0-8) for a matching tool.
/// 3. If not in hotbar, searches main inventory (slots 9-35).
/// 4. Among matches, selects the highest material tier ([`MATERIAL_PRIORITY`] order).
/// 5. If found in main inventory, marks `needs_move_to_hotbar = true`.
/// 6. If no tool is found, returns [`ToolType::Hand`].
pub fn select_tool_for_block(
    block_type: &str,
    inventory: &[Option<ItemStack>],
) -> ToolSelection {
    let required_tool = BLOCK_TO_TOOL_TYPE
        .get(block_type)
        .copied()
        .unwrap_or(ToolType::Hand);

    if required_tool == ToolType::Hand {
        return ToolSelection::hand();
    }

    // Search hotbar first (slots 0-8)
    let hotbar_slice = &inventory[..inventory.len().min(9)];
    if let Some((material, slot)) = find_tool_in_inventory(&required_tool, hotbar_slice) {
        return ToolSelection {
            tool_type: required_tool,
            material: Some(material),
            hotbar_slot: Some(slot),
            needs_move_to_hotbar: false,
        };
    }

    // Search main inventory (slots 9-35)
    if inventory.len() > 9 {
        let main_slice = &inventory[9..inventory.len().min(36)];
        if let Some((material, slot)) = find_tool_in_inventory(&required_tool, main_slice) {
            // slot is relative to main_slice, so add 9 to get absolute slot
            return ToolSelection {
                tool_type: required_tool,
                material: Some(material),
                hotbar_slot: Some(slot + 9),
                needs_move_to_hotbar: true,
            };
        }
    }

    ToolSelection::hand()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_data::ItemStack;
    use crate::types::{MaterialTier, ToolType};

    // ── ToolSelection struct ──────────────────────────────────

    #[test]
    fn test_tool_selection_hand() {
        let sel = ToolSelection::hand();
        assert_eq!(sel.tool_type, ToolType::Hand);
        assert_eq!(sel.material, None);
        assert_eq!(sel.hotbar_slot, None);
        assert!(!sel.needs_move_to_hotbar);
    }

    #[test]
    fn test_tool_selection_with_tool() {
        let sel = ToolSelection {
            tool_type: ToolType::Pickaxe,
            material: Some(MaterialTier::Diamond),
            hotbar_slot: Some(3),
            needs_move_to_hotbar: false,
        };
        assert_eq!(sel.tool_type, ToolType::Pickaxe);
        assert_eq!(sel.material, Some(MaterialTier::Diamond));
        assert_eq!(sel.hotbar_slot, Some(3));
        assert!(!sel.needs_move_to_hotbar);
    }

    // ── material_from_item_name (re-export) ─────────────────

    #[test]
    fn test_material_from_item_name_reexport() {
        assert_eq!(
            material_from_item_name("iron_pickaxe"),
            Some((ToolType::Pickaxe, MaterialTier::Iron))
        );
        assert_eq!(
            material_from_item_name("diamond_axe"),
            Some((ToolType::Axe, MaterialTier::Diamond))
        );
        assert_eq!(
            material_from_item_name("shears"),
            Some((ToolType::Shears, MaterialTier::Iron))
        );
        assert_eq!(material_from_item_name("unknown_item"), None);
    }

    // ── find_tool_in_inventory ────────────────────────────────

    #[test]
    fn test_find_tool_empty_inventory() {
        let inv: Vec<Option<ItemStack>> = vec![];
        assert_eq!(find_tool_in_inventory(&ToolType::Pickaxe, &inv), None);
    }

    #[test]
    fn test_find_tool_none_match() {
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
        assert_eq!(find_tool_in_inventory(&ToolType::Pickaxe, &inv), None);
    }

    #[test]
    fn test_find_tool_selects_highest_tier() {
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
        // iron_pickaxe at slot 3 is higher tier than wooden at slot 1
        assert_eq!(
            find_tool_in_inventory(&ToolType::Pickaxe, &inv),
            Some((MaterialTier::Iron, 3))
        );
    }

    #[test]
    fn test_find_tool_diamond_over_iron() {
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
        // Diamond > Iron > Gold in MATERIAL_PRIORITY
        assert_eq!(
            find_tool_in_inventory(&ToolType::Shovel, &inv),
            Some((MaterialTier::Diamond, 1))
        );
    }

    #[test]
    fn test_find_tool_netherite_best() {
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
        assert_eq!(
            find_tool_in_inventory(&ToolType::Pickaxe, &inv),
            Some((MaterialTier::Netherite, 0))
        );
    }

    #[test]
    fn test_find_tool_returns_slot_and_material() {
        let inv = vec![
            None,
            None,
            Some(ItemStack {
                item_id: "stone_axe".to_string(),
                count: 1,
            }),
        ];
        assert_eq!(
            find_tool_in_inventory(&ToolType::Axe, &inv),
            Some((MaterialTier::Stone, 2))
        );
    }

    #[test]
    fn test_find_tool_shears() {
        let inv = vec![
            Some(ItemStack {
                item_id: "shears".to_string(),
                count: 1,
            }),
        ];
        assert_eq!(
            find_tool_in_inventory(&ToolType::Shears, &inv),
            Some((MaterialTier::Iron, 0))
        );
    }

    // ── select_tool_for_block ─────────────────────────────────

    #[test]
    fn test_select_tool_unknown_block_returns_hand() {
        let inv = vec![];
        let sel = select_tool_for_block("unknown_block", &inv);
        assert_eq!(sel.tool_type, ToolType::Hand);
        assert_eq!(sel.material, None);
        assert_eq!(sel.hotbar_slot, None);
        assert!(!sel.needs_move_to_hotbar);
    }

    #[test]
    fn test_select_tool_hand_block_returns_hand() {
        let inv = vec![];
        let sel = select_tool_for_block("dirt", &inv);
        // dirt requires shovel, but if no inventory, return hand
        // Actually dirt is in BLOCK_TO_TOOL_TYPE as Shovel
        // With empty inventory, should return Hand
        assert_eq!(sel.tool_type, ToolType::Hand);
    }

    #[test]
    fn test_select_tool_found_in_hotbar() {
        let inv = vec![
            Some(ItemStack {
                item_id: "dirt".to_string(),
                count: 1,
            }),
            Some(ItemStack {
                item_id: "iron_pickaxe".to_string(),
                count: 1,
            }),
            Some(ItemStack {
                item_id: "stone".to_string(),
                count: 1,
            }),
        ];
        let sel = select_tool_for_block("stone", &inv);
        assert_eq!(sel.tool_type, ToolType::Pickaxe);
        assert_eq!(sel.material, Some(MaterialTier::Iron));
        assert_eq!(sel.hotbar_slot, Some(1));
        assert!(!sel.needs_move_to_hotbar);
    }

    #[test]
    fn test_select_tool_prefers_higher_tier_in_hotbar() {
        let inv = vec![
            Some(ItemStack {
                item_id: "wooden_pickaxe".to_string(),
                count: 1,
            }),
            Some(ItemStack {
                item_id: "diamond_pickaxe".to_string(),
                count: 1,
            }),
            Some(ItemStack {
                item_id: "iron_pickaxe".to_string(),
                count: 1,
            }),
        ];
        let sel = select_tool_for_block("stone", &inv);
        assert_eq!(sel.tool_type, ToolType::Pickaxe);
        assert_eq!(sel.material, Some(MaterialTier::Diamond));
        assert_eq!(sel.hotbar_slot, Some(1));
        assert!(!sel.needs_move_to_hotbar);
    }

    #[test]
    fn test_select_tool_falls_back_to_main_inventory() {
        let mut inv: Vec<Option<ItemStack>> = vec![None; 36];
        inv[15] = Some(ItemStack {
            item_id: "iron_pickaxe".to_string(),
            count: 1,
        });
        let sel = select_tool_for_block("stone", &inv);
        assert_eq!(sel.tool_type, ToolType::Pickaxe);
        assert_eq!(sel.material, Some(MaterialTier::Iron));
        assert_eq!(sel.hotbar_slot, Some(15));
        assert!(sel.needs_move_to_hotbar);
    }

    #[test]
    fn test_select_tool_prefers_hotbar_over_main() {
        let mut inv: Vec<Option<ItemStack>> = vec![None; 36];
        // Hotbar has wooden pickaxe (slot 0)
        inv[0] = Some(ItemStack {
            item_id: "wooden_pickaxe".to_string(),
            count: 1,
        });
        // Main inventory has diamond pickaxe (slot 20)
        inv[20] = Some(ItemStack {
            item_id: "diamond_pickaxe".to_string(),
            count: 1,
        });
        let sel = select_tool_for_block("stone", &inv);
        // Should prefer hotbar even though main has better tool
        // Wait, the spec says "Search hotbar first, then main inventory"
        // But it also says "Among matches, select highest material tier"
        // This is ambiguous. The spec says:
        // 2. Search hotbar for matching tool
        // 3. If not in hotbar, search main inventory
        // 4. Among matches, select highest material tier
        //
        // I think "Among matches" means among the matches in the searched area.
        // Since we search hotbar first and find a match, we use the best in hotbar.
        // But actually, re-reading: steps 2-4 seem to be:
        // 2. Search hotbar
        // 3. If not in hotbar, search main
        // 4. Among matches (in whichever area was searched), select highest tier
        //
        // So if hotbar has ANY match, we only consider hotbar matches.
        assert_eq!(sel.tool_type, ToolType::Pickaxe);
        assert_eq!(sel.material, Some(MaterialTier::Wood));
        assert_eq!(sel.hotbar_slot, Some(0));
        assert!(!sel.needs_move_to_hotbar);
    }

    #[test]
    fn test_select_tool_main_inventory_best_tier() {
        let mut inv: Vec<Option<ItemStack>> = vec![None; 36];
        inv[10] = Some(ItemStack {
            item_id: "stone_pickaxe".to_string(),
            count: 1,
        });
        inv[25] = Some(ItemStack {
            item_id: "iron_pickaxe".to_string(),
            count: 1,
        });
        let sel = select_tool_for_block("stone", &inv);
        assert_eq!(sel.tool_type, ToolType::Pickaxe);
        assert_eq!(sel.material, Some(MaterialTier::Iron));
        assert_eq!(sel.hotbar_slot, Some(25));
        assert!(sel.needs_move_to_hotbar);
    }

    #[test]
    fn test_select_tool_no_matching_tool_returns_hand() {
        let inv = vec![
            Some(ItemStack {
                item_id: "dirt".to_string(),
                count: 1,
            }),
            Some(ItemStack {
                item_id: "oak_planks".to_string(),
                count: 1,
            }),
        ];
        let sel = select_tool_for_block("stone", &inv);
        assert_eq!(sel.tool_type, ToolType::Hand);
        assert_eq!(sel.material, None);
        assert_eq!(sel.hotbar_slot, None);
        assert!(!sel.needs_move_to_hotbar);
    }

    #[test]
    fn test_select_tool_axe_block() {
        let inv = vec![
            Some(ItemStack {
                item_id: "iron_axe".to_string(),
                count: 1,
            }),
        ];
        let sel = select_tool_for_block("oak_log", &inv);
        assert_eq!(sel.tool_type, ToolType::Axe);
        assert_eq!(sel.material, Some(MaterialTier::Iron));
        assert_eq!(sel.hotbar_slot, Some(0));
        assert!(!sel.needs_move_to_hotbar);
    }

    #[test]
    fn test_select_tool_shovel_block() {
        let inv = vec![
            Some(ItemStack {
                item_id: "diamond_shovel".to_string(),
                count: 1,
            }),
        ];
        let sel = select_tool_for_block("dirt", &inv);
        assert_eq!(sel.tool_type, ToolType::Shovel);
        assert_eq!(sel.material, Some(MaterialTier::Diamond));
        assert_eq!(sel.hotbar_slot, Some(0));
        assert!(!sel.needs_move_to_hotbar);
    }

    #[test]
    fn test_select_tool_shears_block() {
        let inv = vec![
            Some(ItemStack {
                item_id: "shears".to_string(),
                count: 1,
            }),
        ];
        let sel = select_tool_for_block("white_wool", &inv);
        assert_eq!(sel.tool_type, ToolType::Shears);
        assert_eq!(sel.material, Some(MaterialTier::Iron));
        assert_eq!(sel.hotbar_slot, Some(0));
        assert!(!sel.needs_move_to_hotbar);
    }

    #[test]
    fn test_select_tool_small_inventory() {
        // Inventory with only 5 slots (all hotbar)
        let inv = vec![
            None,
            Some(ItemStack {
                item_id: "stone_pickaxe".to_string(),
                count: 1,
            }),
        ];
        let sel = select_tool_for_block("stone", &inv);
        assert_eq!(sel.tool_type, ToolType::Pickaxe);
        assert_eq!(sel.material, Some(MaterialTier::Stone));
        assert_eq!(sel.hotbar_slot, Some(1));
        assert!(!sel.needs_move_to_hotbar);
    }

    #[test]
    fn test_select_tool_main_inventory_slot_mapping() {
        // 36-slot inventory, tool at slot 9 (first main inventory slot)
        let mut inv: Vec<Option<ItemStack>> = vec![None; 36];
        inv[9] = Some(ItemStack {
            item_id: "iron_axe".to_string(),
            count: 1,
        });
        let sel = select_tool_for_block("oak_log", &inv);
        assert_eq!(sel.hotbar_slot, Some(9));
        assert!(sel.needs_move_to_hotbar);
    }
}
