//! Tool / inventory selection logic for mining and combat.

use crate::block_data::{
    BLOCK_TO_TOOL_TYPE, ItemStack, MATERIAL_PRIORITY, harvest_level_of,
    minimum_material_for_harvest_level,
};
use crate::types::{MaterialTier, ToolType};

/// The result of selecting a tool for a specific block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSelection {
    pub tool_type: ToolType,
    pub material: Option<MaterialTier>,
    pub hotbar_slot: Option<u8>,
    pub needs_move_to_hotbar: bool,
    pub required_harvest_level: Option<u8>,
    /// The `item_id` (e.g. `"iron_pickaxe"`) of the matched inventory entry.
    ///
    /// Populated in BOTH the hotbar and main-inventory branches so the
    /// compound-op executor can issue a `MoveItemToHotbar` for a
    /// main-inventory tool (audit H-1). `None` for a `Hand` selection.
    pub item_id: Option<String>,
}

impl ToolSelection {
    /// Create a `Hand` selection when no tool is available.
    pub fn hand() -> Self {
        Self {
            tool_type: ToolType::Hand,
            material: None,
            hotbar_slot: None,
            needs_move_to_hotbar: false,
            required_harvest_level: None,
            item_id: None,
        }
    }
}

/// Re-export of [`crate::block_data::material_from_item_name`].
pub use crate::block_data::material_from_item_name;

/// Finds the best tool of the given type anywhere in the inventory.
///
/// Returns `(material_tier, slot_index)` for the highest-tier match,
/// or `None` if no matching tool is found.
///
/// If `required_harvest_level` is `Some(level)`, tools whose material tier's
/// harvest level (see [`crate::block_data::harvest_level_of`]) is **below**
/// `level` are filtered out — a wooden pickaxe cannot mine diamond_ore
/// (which needs iron+), so the function returns `None` even if the wooden
/// pickaxe is the only pickaxe in the inventory. Pass `None` to disable the
/// harvest-level check (back-compat for callers that don't care).
pub fn find_tool_in_inventory(
    tool_type: &ToolType,
    inventory: &[Option<ItemStack>],
    required_harvest_level: Option<u8>,
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

            // Filter out tools whose harvest level is too low to drop the
            // target block. This prevents a wood pickaxe from being
            // optimistically returned for diamond_ore, which would silently
            // drop nothing in-game.
            if let Some(req) = required_harvest_level
                && harvest_level_of(found_material) < req
            {
                continue;
            }

            let priority = MATERIAL_PRIORITY.iter().position(|m| m == &found_material);

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
/// 2. Looks up the required harvest level from [`crate::block_data::HARVEST_LEVEL`].
///    Tools below this level are filtered out (a wood pickaxe can't mine diamond_ore).
/// 3. Searches hotbar (slots 0-8) for a matching tool.
/// 4. If not in hotbar, searches main inventory (slots 9-35).
/// 5. Among matches, selects the highest material tier ([`MATERIAL_PRIORITY`] order).
/// 6. If found in main inventory, marks `needs_move_to_hotbar = true`.
/// 7. If no tool is found, returns [`ToolType::Hand`].
pub fn select_tool_for_block(block_type: &str, inventory: &[Option<ItemStack>]) -> ToolSelection {
    let primary_tool = BLOCK_TO_TOOL_TYPE
        .get(block_type)
        .copied()
        .unwrap_or(ToolType::Hand);

    if primary_tool == ToolType::Hand {
        return ToolSelection::hand();
    }

    // Pass the block's required harvest level down so under-tier tools are
    // rejected. None for unknown blocks (no entry) means "no harvest check".
    let required_harvest_level = crate::block_data::HARVEST_LEVEL.get(block_type).copied();

    // Search hotbar first (slots 0-8), then the main inventory (slots 9-35).
    // A tool found in the main inventory can't be switched to directly —
    // SwitchHotbarSlot only accepts hotbar indices (0-8) — so we surface the
    // tool type/material but leave `hotbar_slot` as None, and mark
    // `needs_move_to_hotbar` so the compound-op executor issues a
    // `MoveItemToHotbar` before mining (H-1).
    //
    // Candidates are tried in priority order: the block's primary tool,
    // then an alternative tool when one exists (2026-08-30 review — cobweb
    // accepts shears alongside the sword primary, since vanilla gives both
    // the same break speed; a shears-only bot must not be refused).
    let candidates = std::iter::once(primary_tool).chain(
        crate::block_data::ALT_TOOL_FOR_BLOCK
            .get(block_type)
            .copied(),
    );
    for required_tool in candidates {
        let hotbar_slice = &inventory[..inventory.len().min(9)];
        if let Some((material, slot)) =
            find_tool_in_inventory(&required_tool, hotbar_slice, required_harvest_level)
        {
            let item_id = hotbar_slice
                .get(slot as usize)
                .and_then(|opt| opt.as_ref())
                .map(|stack| stack.item_id.clone());
            return ToolSelection {
                tool_type: required_tool,
                material: Some(material),
                hotbar_slot: Some(slot),
                needs_move_to_hotbar: false,
                required_harvest_level,
                item_id,
            };
        }

        if inventory.len() > 9 {
            let main_slice = &inventory[9..inventory.len().min(36)];
            if let Some((material, slot)) =
                find_tool_in_inventory(&required_tool, main_slice, required_harvest_level)
            {
                let item_id = main_slice
                    .get(slot as usize)
                    .and_then(|opt| opt.as_ref())
                    .map(|stack| stack.item_id.clone());
                return ToolSelection {
                    tool_type: required_tool,
                    material: Some(material),
                    hotbar_slot: None,
                    needs_move_to_hotbar: true,
                    required_harvest_level,
                    item_id,
                };
            }
        }
    }

    ToolSelection {
        required_harvest_level,
        ..ToolSelection::hand()
    }
}

/// Builds a human-readable list of suggested alternative tools.
///
/// Returns a vector of strings like `["Iron Pickaxe"]` suggesting the minimum
/// tier tool that meets the harvest level requirement. Returns an empty vec
/// when no specific tool is required (level 0 / unknown block) — level 0
/// means "any tool (or hand) works", so there is no tool to suggest.
pub fn build_tool_alternatives(
    tool_type: ToolType,
    required_harvest_level: Option<u8>,
) -> Vec<String> {
    let mut alts = Vec::new();
    if let Some(level) = required_harvest_level
        && level > 0
        && let Some(mat) = minimum_material_for_harvest_level(level)
    {
        alts.push(format!("{mat} {tool_type}"));
    }
    alts
}

// ---------------------------------------------------------------------------

/// Alternatives for a block that needs a SPECIFIC tool but has no tier
/// requirement (harvest level 0 + requires_correct_tool_for_drops, e.g.
/// cobbled_deepslate): the weakest tool of the type is sufficient.
///
/// Shears have no material prefix; Hand never needs an alternative.
pub fn base_tool_alternative(tool_type: ToolType) -> Vec<String> {
    match tool_type {
        ToolType::Shears => vec!["shears".into()],
        ToolType::Hand => vec![],
        other => vec![format!("{} {other}", MaterialTier::Wood)],
    }
}
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_data::ItemStack;
    use crate::types::{MaterialTier, ToolType};

    #[test]
    fn test_select_tool_for_block_uses_alt_when_primary_missing() {
        // 2026-08-30 review: cobweb's primary is Sword, but a shears-only
        // inventory must select the shears (same vanilla break speed)
        // instead of falling through to Hand and being refused.
        let inventory = vec![Some(ItemStack {
            item_id: "shears".into(),
            count: 1,
        })];
        let selection = select_tool_for_block("cobweb", &inventory);
        assert_eq!(selection.tool_type, ToolType::Shears);
        assert_eq!(selection.hotbar_slot, Some(0));
        assert!(!selection.needs_move_to_hotbar);

        // Primary still wins when both are present.
        let both = vec![
            Some(ItemStack {
                item_id: "shears".into(),
                count: 1,
            }),
            Some(ItemStack {
                item_id: "iron_sword".into(),
                count: 1,
            }),
        ];
        let selection = select_tool_for_block("cobweb", &both);
        assert_eq!(selection.tool_type, ToolType::Sword);
        assert_eq!(selection.hotbar_slot, Some(1));

        // Neither present → hand fallback (the caller decides refusal).
        let empty = vec![None, None];
        let selection = select_tool_for_block("cobweb", &empty);
        assert_eq!(selection.tool_type, ToolType::Hand);
    }

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
            required_harvest_level: None,
            item_id: Some("diamond_pickaxe".into()),
        };
        assert_eq!(sel.tool_type, ToolType::Pickaxe);
        assert_eq!(sel.material, Some(MaterialTier::Diamond));
        assert_eq!(sel.hotbar_slot, Some(3));
        assert!(!sel.needs_move_to_hotbar);
        assert_eq!(sel.item_id.as_deref(), Some("diamond_pickaxe"));
    }

    // ── ToolSelection.item_id (audit H-1) ───────────────────

    #[test]
    fn test_tool_selection_item_id_from_hotbar_match() {
        let inv = vec![
            Some(ItemStack {
                item_id: "iron_pickaxe".to_string(),
                count: 1,
            }),
            Some(ItemStack {
                item_id: "dirt".to_string(),
                count: 1,
            }),
        ];
        let sel = select_tool_for_block("stone", &inv);
        assert_eq!(sel.tool_type, ToolType::Pickaxe);
        assert_eq!(sel.hotbar_slot, Some(0));
        assert!(!sel.needs_move_to_hotbar);
        assert_eq!(sel.item_id.as_deref(), Some("iron_pickaxe"));
    }

    #[test]
    fn test_tool_selection_item_id_from_main_inventory_match() {
        // Iron pickaxe in the MAIN inventory (slot 15): the selection must
        // carry the matched entry's item_id so the compound-op executor can
        // dispatch `MoveItemToHotbar` (audit H-1). `hotbar_slot` stays None.
        let mut inv: Vec<Option<ItemStack>> = vec![None; 36];
        inv[15] = Some(ItemStack {
            item_id: "iron_pickaxe".to_string(),
            count: 1,
        });
        let sel = select_tool_for_block("stone", &inv);
        assert_eq!(sel.tool_type, ToolType::Pickaxe);
        assert_eq!(sel.material, Some(MaterialTier::Iron));
        assert_eq!(sel.hotbar_slot, None);
        assert!(sel.needs_move_to_hotbar);
        assert_eq!(sel.item_id.as_deref(), Some("iron_pickaxe"));
    }

    #[test]
    fn test_tool_selection_item_id_none_for_hand() {
        let sel = select_tool_for_block("stone", &[]);
        assert_eq!(sel.tool_type, ToolType::Hand);
        assert_eq!(sel.item_id, None);
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
        assert_eq!(find_tool_in_inventory(&ToolType::Pickaxe, &inv, None), None);
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
        assert_eq!(find_tool_in_inventory(&ToolType::Pickaxe, &inv, None), None);
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
            find_tool_in_inventory(&ToolType::Pickaxe, &inv, None),
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
            find_tool_in_inventory(&ToolType::Shovel, &inv, None),
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
            find_tool_in_inventory(&ToolType::Pickaxe, &inv, None),
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
            find_tool_in_inventory(&ToolType::Axe, &inv, None),
            Some((MaterialTier::Stone, 2))
        );
    }

    #[test]
    fn test_find_tool_shears() {
        let inv = vec![Some(ItemStack {
            item_id: "shears".to_string(),
            count: 1,
        })];
        assert_eq!(
            find_tool_in_inventory(&ToolType::Shears, &inv, None),
            Some((MaterialTier::Iron, 0))
        );
    }

    // ── find_tool_in_inventory harvest-level filter ──────────

    #[test]
    fn test_harvest_level_wood_cant_mine_diamond() {
        // diamond_ore requires harvest level 2 (iron+). A wood pickaxe has
        // level 0 and must be rejected.
        let inv = vec![Some(ItemStack {
            item_id: "wooden_pickaxe".to_string(),
            count: 1,
        })];
        assert_eq!(
            find_tool_in_inventory(&ToolType::Pickaxe, &inv, Some(2)),
            None
        );
    }

    #[test]
    fn test_harvest_level_diamond_mine_diamond_ore() {
        // diamond_pickaxe has level 3 — comfortably above diamond_ore's
        // required level 2.
        let inv = vec![Some(ItemStack {
            item_id: "diamond_pickaxe".to_string(),
            count: 1,
        })];
        assert_eq!(
            find_tool_in_inventory(&ToolType::Pickaxe, &inv, Some(2)),
            Some((MaterialTier::Diamond, 0))
        );
    }

    #[test]
    fn test_harvest_level_wood_cant_mine_lapis_block() {
        // 2026-08-29 review: lapis_block is vanilla needs_stone_tool
        // (level 1). The old level-0 entry accepted a wood pickaxe, which
        // breaks the block for NO drops. Stone tier is required.
        let inv = vec![Some(ItemStack {
            item_id: "wooden_pickaxe".to_string(),
            count: 1,
        })];
        assert_eq!(
            find_tool_in_inventory(&ToolType::Pickaxe, &inv, Some(1)),
            None
        );
        // A stone pickaxe passes.
        let inv = vec![Some(ItemStack {
            item_id: "stone_pickaxe".to_string(),
            count: 1,
        })];
        assert_eq!(
            find_tool_in_inventory(&ToolType::Pickaxe, &inv, Some(1)),
            Some((MaterialTier::Stone, 0))
        );
    }

    #[test]
    fn test_harvest_level_diamond_pickaxe_mines_netherite_block() {
        // 2026-08-29 review: netherite_block sits at level 3 (vanilla
        // needs_diamond_tool; needs_netherite_tool is empty), so a diamond
        // pickaxe must NOT be refused anymore.
        let inv = vec![Some(ItemStack {
            item_id: "diamond_pickaxe".to_string(),
            count: 1,
        })];
        assert_eq!(
            find_tool_in_inventory(&ToolType::Pickaxe, &inv, Some(3)),
            Some((MaterialTier::Diamond, 0))
        );
    }

    #[test]
    fn test_harvest_level_iron_passes_diamond_filter() {
        // iron_pickaxe has level 2 — exactly meets diamond_ore's requirement.
        let inv = vec![Some(ItemStack {
            item_id: "iron_pickaxe".to_string(),
            count: 1,
        })];
        assert_eq!(
            find_tool_in_inventory(&ToolType::Pickaxe, &inv, Some(2)),
            Some((MaterialTier::Iron, 0))
        );
    }

    #[test]
    fn test_harvest_level_stone_fails_diamond_filter() {
        // stone_pickaxe has level 1, below diamond_ore's required 2.
        let inv = vec![Some(ItemStack {
            item_id: "stone_pickaxe".to_string(),
            count: 1,
        })];
        assert_eq!(
            find_tool_in_inventory(&ToolType::Pickaxe, &inv, Some(2)),
            None
        );
    }

    #[test]
    fn test_harvest_level_filter_picks_highest_qualifying() {
        // Among pickaxes of varying levels, only those with level >= 2 count.
        // The best pickaxe that passes the filter is iron (level 2), since
        // wooden (0) and stone (1) are filtered out.
        let inv = vec![
            Some(ItemStack {
                item_id: "wooden_pickaxe".to_string(),
                count: 1,
            }),
            Some(ItemStack {
                item_id: "iron_pickaxe".to_string(),
                count: 1,
            }),
            Some(ItemStack {
                item_id: "stone_pickaxe".to_string(),
                count: 1,
            }),
        ];
        assert_eq!(
            find_tool_in_inventory(&ToolType::Pickaxe, &inv, Some(2)),
            Some((MaterialTier::Iron, 1))
        );
    }

    #[test]
    fn test_harvest_level_filter_none_disables_check() {
        // None means no filter — even wood pickaxe is returned.
        let inv = vec![Some(ItemStack {
            item_id: "wooden_pickaxe".to_string(),
            count: 1,
        })];
        assert_eq!(
            find_tool_in_inventory(&ToolType::Pickaxe, &inv, None),
            Some((MaterialTier::Wood, 0))
        );
    }

    // ── select_tool_for_block harvest-level interaction ──────

    #[test]
    fn test_select_tool_rejects_wood_pickaxe_for_diamond_ore() {
        // Even though select_tool_for_block finds wooden_pickaxe first, the
        // harvest-level check should reject it for diamond_ore (level 2).
        let inv = vec![Some(ItemStack {
            item_id: "wooden_pickaxe".to_string(),
            count: 1,
        })];
        let sel = select_tool_for_block("diamond_ore", &inv);
        assert_eq!(sel.tool_type, ToolType::Hand);
        assert_eq!(sel.material, None);
        assert_eq!(sel.hotbar_slot, None);
    }

    #[test]
    fn test_select_tool_iron_pickaxe_for_diamond_ore() {
        let inv = vec![Some(ItemStack {
            item_id: "iron_pickaxe".to_string(),
            count: 1,
        })];
        let sel = select_tool_for_block("diamond_ore", &inv);
        assert_eq!(sel.tool_type, ToolType::Pickaxe);
        assert_eq!(sel.material, Some(MaterialTier::Iron));
    }

    #[test]
    fn test_select_tool_diamond_pickaxe_for_ancient_debris() {
        // Ancient debris requires harvest level 3 (diamond+). A diamond
        // pickaxe must be accepted — regression for the earlier bug that
        // required netherite (level 4) and wrongly rejected diamond.
        let inv = vec![Some(ItemStack {
            item_id: "diamond_pickaxe".to_string(),
            count: 1,
        })];
        let sel = select_tool_for_block("ancient_debris", &inv);
        assert_eq!(sel.tool_type, ToolType::Pickaxe);
        assert_eq!(sel.material, Some(MaterialTier::Diamond));
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
        // Main-inventory tools can't be switched to directly, so no hotbar slot.
        assert_eq!(sel.hotbar_slot, None);
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
        // The new Harvest Level rules (H-1) require at least a stone
        // pickaxe (harvest level 1) to mine `stone` — wood pickaxe
        // (level 0) is rejected. The tool selector must therefore
        // fall through to the main inventory's diamond pickaxe,
        // even though the hotbar has *a* pickaxe. This is the
        // "level gate" override: harvest-level mismatch trumps
        // hotbar preference.
        assert_eq!(sel.tool_type, ToolType::Pickaxe);
        assert_eq!(sel.material, Some(MaterialTier::Diamond));
        assert_eq!(sel.hotbar_slot, None);
        assert!(sel.needs_move_to_hotbar);
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
        assert_eq!(sel.hotbar_slot, None);
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
        let inv = vec![Some(ItemStack {
            item_id: "iron_axe".to_string(),
            count: 1,
        })];
        let sel = select_tool_for_block("oak_log", &inv);
        assert_eq!(sel.tool_type, ToolType::Axe);
        assert_eq!(sel.material, Some(MaterialTier::Iron));
        assert_eq!(sel.hotbar_slot, Some(0));
        assert!(!sel.needs_move_to_hotbar);
    }

    #[test]
    fn test_select_tool_shovel_block() {
        let inv = vec![Some(ItemStack {
            item_id: "diamond_shovel".to_string(),
            count: 1,
        })];
        let sel = select_tool_for_block("dirt", &inv);
        assert_eq!(sel.tool_type, ToolType::Shovel);
        assert_eq!(sel.material, Some(MaterialTier::Diamond));
        assert_eq!(sel.hotbar_slot, Some(0));
        assert!(!sel.needs_move_to_hotbar);
    }

    #[test]
    fn test_select_tool_shears_block() {
        let inv = vec![Some(ItemStack {
            item_id: "shears".to_string(),
            count: 1,
        })];
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
        // Tool at first main-inventory slot (9) — can't be selected directly.
        assert_eq!(sel.hotbar_slot, None);
        assert!(sel.needs_move_to_hotbar);
        // H-1: item_id must be populated from the main-inventory match too.
        assert_eq!(sel.item_id.as_deref(), Some("iron_axe"));
    }

    // ── build_tool_alternatives (audit L-33 coverage) ──────────

    #[test]
    fn test_build_tool_alternatives_maps_level_to_tier() {
        // Level 2 → Iron. The string is the minimum material + tool type in
        // LLM-readable form ("iron pickaxe").
        assert_eq!(
            build_tool_alternatives(ToolType::Pickaxe, Some(2)),
            vec!["iron pickaxe"]
        );
        assert_eq!(
            build_tool_alternatives(ToolType::Pickaxe, Some(3)),
            vec!["diamond pickaxe"]
        );
        assert_eq!(
            build_tool_alternatives(ToolType::Pickaxe, Some(1)),
            vec!["stone pickaxe"]
        );
        assert_eq!(
            build_tool_alternatives(ToolType::Pickaxe, Some(4)),
            vec!["netherite pickaxe"]
        );
    }

    #[test]
    fn test_build_tool_alternatives_no_requirement_is_empty() {
        // Level 0 / unknown → no alternative tool is suggested.
        assert!(build_tool_alternatives(ToolType::Pickaxe, None).is_empty());
        assert!(build_tool_alternatives(ToolType::Pickaxe, Some(0)).is_empty());
        // Out-of-range levels map to no material → empty too.
        assert!(build_tool_alternatives(ToolType::Pickaxe, Some(5)).is_empty());
    }

    #[test]
    fn test_build_tool_alternatives_string_format() {
        // Format is "{material} {tool_type}" (Display of MaterialTier +
        // ToolType), e.g. "stone pickaxe" — matching the historical
        // LLM-facing "use an Iron Pickaxe" guidance style.
        let alts = build_tool_alternatives(ToolType::Shovel, Some(2));
        assert_eq!(alts, vec!["iron shovel"]);
        let alts = build_tool_alternatives(ToolType::Axe, Some(1));
        assert_eq!(alts, vec!["stone axe"]);
    }
}
