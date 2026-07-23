//! Block and tool data tables for the Minecraft MCP server.
//!
//! Provides lookup functions for block-to-tool mappings, material tier speeds,
//! tool name patterns, block hardness values, and mining time calculations.
//!
//! > **Note:** Most items in this module are lookup tables designed for the
//! > bot ops layer.  They are retained for the integration plan.

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
        // Netherite blocks
        "ancient_debris",
        "netherite_block",
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
        "vine",
        "glow_lichen",
    ] {
        m.insert(block, ToolType::Shears);
    }

    // --- Sword blocks ---
    // Swords are the fastest tool for cobweb in Java Edition (faster than shears).
    m.insert("cobweb", ToolType::Sword);

    // --- Hoe blocks ---
    // Hoes are the fastest tool for hay bales, sculk, and moss blocks.
    // (Leaves variants are already mapped to Shears above.)
    for &block in &["hay_bale", "sculk", "moss_block"] {
        m.insert(block, ToolType::Hoe);
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

        m.insert((ToolType::Sword, MaterialTier::Wood), vec!["wooden_sword"]);
        m.insert((ToolType::Sword, MaterialTier::Stone), vec!["stone_sword"]);
        m.insert((ToolType::Sword, MaterialTier::Iron), vec!["iron_sword"]);
        m.insert((ToolType::Sword, MaterialTier::Gold), vec!["golden_sword"]);
        m.insert(
            (ToolType::Sword, MaterialTier::Diamond),
            vec!["diamond_sword"],
        );
        m.insert(
            (ToolType::Sword, MaterialTier::Netherite),
            vec!["netherite_sword"],
        );

        m.insert((ToolType::Hoe, MaterialTier::Wood), vec!["wooden_hoe"]);
        m.insert((ToolType::Hoe, MaterialTier::Stone), vec!["stone_hoe"]);
        m.insert((ToolType::Hoe, MaterialTier::Iron), vec!["iron_hoe"]);
        m.insert((ToolType::Hoe, MaterialTier::Gold), vec!["golden_hoe"]);
        m.insert((ToolType::Hoe, MaterialTier::Diamond), vec!["diamond_hoe"]);
        m.insert(
            (ToolType::Hoe, MaterialTier::Netherite),
            vec!["netherite_hoe"],
        );

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

    // Stone variants (stairs, slabs, walls)
    m.insert("cobblestone_stairs", 2.0);
    m.insert("cobblestone_slab", 2.0);
    m.insert("cobblestone_wall", 2.0);
    m.insert("stone_stairs", 1.5);
    m.insert("stone_slab", 1.5);
    m.insert("mossy_stone_bricks", 1.5);
    m.insert("cracked_stone_bricks", 1.5);
    m.insert("brick_stairs", 2.0);
    m.insert("brick_slab", 2.0);
    m.insert("bricks", 2.0);

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

    // Deepslate variants
    m.insert("deepslate_emerald_ore", 4.5);
    m.insert("deepslate_lapis_ore", 4.5);
    m.insert("deepslate_redstone_ore", 4.5);

    // Nether / End
    m.insert("netherrack", 0.4);
    m.insert("nether_quartz_ore", 3.0);
    m.insert("nether_gold_ore", 3.0);
    m.insert("end_stone", 3.0);
    m.insert("purpur_block", 3.0);
    m.insert("purpur_pillar", 3.0);

    // Wood
    m.insert("oak_log", 2.0);
    m.insert("spruce_log", 2.0);
    m.insert("birch_log", 2.0);
    m.insert("jungle_log", 2.0);
    m.insert("acacia_log", 2.0);
    m.insert("dark_oak_log", 2.0);
    m.insert("oak_planks", 2.0);
    m.insert("spruce_planks", 2.0);
    m.insert("birch_planks", 2.0);
    m.insert("jungle_planks", 2.0);
    m.insert("acacia_planks", 2.0);
    m.insert("dark_oak_planks", 2.0);
    m.insert("oak_stairs", 2.0);
    m.insert("spruce_stairs", 2.0);
    m.insert("birch_stairs", 2.0);
    m.insert("oak_slab", 2.0);
    m.insert("spruce_slab", 2.0);
    m.insert("birch_slab", 2.0);
    m.insert("oak_fence", 2.0);
    m.insert("spruce_fence", 2.0);
    m.insert("birch_fence", 2.0);
    m.insert("oak_fence_gate", 2.0);
    m.insert("oak_door", 3.0);
    m.insert("spruce_door", 3.0);
    m.insert("birch_door", 3.0);
    m.insert("crafting_table", 2.5);
    m.insert("bookshelf", 1.5);
    m.insert("ladder", 0.4);
    m.insert("barrel", 2.5);

    // Dirt & sand
    m.insert("dirt", 0.5);
    m.insert("grass_block", 0.6);
    m.insert("coarse_dirt", 0.5);
    m.insert("rooted_dirt", 0.5);
    m.insert("dirt_path", 0.65);
    m.insert("sand", 0.5);
    m.insert("red_sand", 0.5);
    m.insert("suspicious_sand", 0.25);
    m.insert("gravel", 0.6);
    m.insert("clay", 0.6);
    m.insert("mud", 0.5);
    m.insert("muddy_mangrove_roots", 0.7);
    m.insert("soul_sand", 0.5);
    m.insert("soul_soil", 0.5);
    m.insert("farmland", 0.6);
    m.insert("mycelium", 0.6);
    m.insert("podzol", 0.5);
    m.insert("powder_snow", 0.1);
    m.insert("snow_block", 0.2);

    // Leaves & plants
    m.insert("oak_leaves", 0.2);
    m.insert("spruce_leaves", 0.2);
    m.insert("birch_leaves", 0.2);
    m.insert("jungle_leaves", 0.2);
    m.insert("acacia_leaves", 0.2);
    m.insert("dark_oak_leaves", 0.2);
    m.insert("azalea_leaves", 0.2);
    m.insert("vine", 0.2);
    m.insert("glow_lichen", 0.2);
    m.insert("moss_block", 0.1);
    m.insert("sculk", 0.2);

    // Wool & glass
    m.insert("white_wool", 0.8);
    m.insert("orange_wool", 0.8);
    m.insert("magenta_wool", 0.8);
    m.insert("light_blue_wool", 0.8);
    m.insert("yellow_wool", 0.8);
    m.insert("lime_wool", 0.8);
    m.insert("pink_wool", 0.8);
    m.insert("gray_wool", 0.8);
    m.insert("light_gray_wool", 0.8);
    m.insert("cyan_wool", 0.8);
    m.insert("purple_wool", 0.8);
    m.insert("blue_wool", 0.8);
    m.insert("brown_wool", 0.8);
    m.insert("green_wool", 0.8);
    m.insert("red_wool", 0.8);
    m.insert("black_wool", 0.8);
    m.insert("glass", 0.3);
    m.insert("glass_pane", 0.3);
    m.insert("white_stained_glass", 0.3);
    m.insert("white_stained_glass_pane", 0.3);

    // Other
    m.insert("ice", 0.5);
    m.insert("packed_ice", 0.5);
    m.insert("blue_ice", 0.5);
    m.insert("snow", 0.1);
    m.insert("hay_bale", 0.5);
    m.insert("cobweb", 4.0);

    // Notable blocks
    m.insert("obsidian", 50.0);
    m.insert("furnace", 3.5);
    m.insert("blast_furnace", 3.5);
    m.insert("smoker", 3.5);
    m.insert("anvil", 5.0);
    m.insert("chipped_anvil", 5.0);
    m.insert("damaged_anvil", 5.0);
    m.insert("enchanting_table", 5.0);
    m.insert("brewing_stand", 0.5);
    m.insert("hopper", 3.0);
    m.insert("dropper", 3.5);
    m.insert("dispenser", 3.5);
    m.insert("observer", 3.0);
    m.insert("ender_chest", 22.5);
    m.insert("chest", 2.5);
    m.insert("trapped_chest", 2.5);
    m.insert("iron_block", 5.0);
    m.insert("gold_block", 5.0);
    m.insert("diamond_block", 5.0);
    m.insert("emerald_block", 5.0);
    m.insert("netherite_block", 50.0);

    // Ancient debris (needs diamond+ pickaxe, drops as raw ancient debris)
    m.insert("ancient_debris", 30.0);

    m
});

/// Material tier priority order — from best (index 0) to worst (index N).
///
/// Used by [`find_best_tool_in_inventory`] to select the highest-tier tool.
/// This is the reverse of the `Ord` derive on [`MaterialTier`] (whose variant
/// order is `Wood < Gold < Stone < Iron < Diamond < Netherite`), so the
/// highest-`Ord` tier is preferred. Gold ranks above Wood (it has the same
/// mining level but higher speed), and below Stone (lower durability and
/// mining level).
pub static MATERIAL_PRIORITY: &[MaterialTier] = &[
    MaterialTier::Netherite,
    MaterialTier::Diamond,
    MaterialTier::Iron,
    MaterialTier::Stone,
    MaterialTier::Gold,
    MaterialTier::Wood,
];

/// Minecraft harvest level for each [`MaterialTier`].
///
/// Levels: Wood=0, Gold/Stone=1, Iron=2, Diamond=3, Netherite=4. Gold shares
/// Stone's level (mines the same set of blocks, just faster and with less
/// durability). Used by [`crate::tool_select::find_tool_in_inventory`] to
/// filter out tools that are too weak to drop the target block.
pub fn harvest_level_of(material: MaterialTier) -> u8 {
    match material {
        MaterialTier::Wood => 0,
        MaterialTier::Gold | MaterialTier::Stone => 1,
        MaterialTier::Iron => 2,
        MaterialTier::Diamond => 3,
        MaterialTier::Netherite => 4,
    }
}

/// Returns the minimum [`MaterialTier`] required to meet a given harvest level.
///
/// Returns `None` if the level exceeds the highest known tier (Netherite = 4).
pub fn minimum_material_for_harvest_level(level: u8) -> Option<MaterialTier> {
    match level {
        0 => Some(MaterialTier::Wood),
        1 => Some(MaterialTier::Stone),
        2 => Some(MaterialTier::Iron),
        3 => Some(MaterialTier::Diamond),
        4 => Some(MaterialTier::Netherite),
        _ => None,
    }
}

/// Required harvest level for a block type.
///
/// The level means: "the tool must have at least this harvest level to make
/// the block drop its item". Below the required level the block is mined
/// (slowly, by hand), but drops nothing.
///
/// Defaults to 0 for unknown blocks — any tool will do. The values here
/// mirror the vanilla Minecraft 1.21 harvest rules for the blocks the bot
/// knows about. Blocks not in this table are assumed to be hand-mineable.
pub static HARVEST_LEVEL: LazyLock<HashMap<&'static str, u8>> = LazyLock::new(|| {
    let mut m = HashMap::new();

    // Level 0: hand-mineable (wood, dirt, sand, most plants).
    // Wood-family blocks: any axe is fine, hand drops the block.
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
        "oak_slab",
        "oak_fence",
        "oak_fence_gate",
        "crafting_table",
        "bookshelf",
        "ladder",
        "barrel",
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
        "oak_leaves",
        "spruce_leaves",
        "birch_leaves",
        "jungle_leaves",
        "acacia_leaves",
        "dark_oak_leaves",
        "azalea_leaves",
        "white_wool",
        "cobweb",
        "hay_bale",
        "sculk",
        "moss_block",
        "vine",
        "glow_lichen",
        "glass",
        "glass_pane",
        "white_stained_glass",
        "white_stained_glass_pane",
        "ice",
    ] {
        m.insert(block, 0u8);
    }

    // Level 1: needs stone+ (cobblestone, stone, netherrack, stone bricks,
    // most stone-family blocks, all 0-level ores like coal/iron/copper, etc.).
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
        "tuff",
        "calcite",
        "netherrack",
        "nether_quartz_ore",
        "nether_gold_ore",
        "end_stone",
        "purpur_block",
        "purpur_pillar",
        "bricks",
        "brick_slab",
        "brick_stairs",
        "furnace",
        "blast_furnace",
        "smoker",
        "enchanting_table",
        "brewing_stand",
        "hopper",
        "dropper",
        "dispenser",
        "observer",
        "chest",
        "trapped_chest",
        "ender_chest",
        "deepslate",
        "coal_ore",
        "deepslate_coal_ore",
        "iron_ore",
        "deepslate_iron_ore",
        "copper_ore",
        "deepslate_copper_ore",
        "lapis_ore",
        "deepslate_lapis_ore",
    ] {
        m.insert(block, 1u8);
    }

    // Level 2: needs iron+ (gold, diamond, emerald ores; deepslate variants;
    // iron/diamond/blocks; anvils; obsidian-adjacent blocks).
    for &block in &[
        "gold_ore",
        "deepslate_gold_ore",
        "redstone_ore",
        "deepslate_redstone_ore",
        "diamond_ore",
        "deepslate_diamond_ore",
        "emerald_ore",
        "deepslate_emerald_ore",
        "iron_block",
        "gold_block",
        "diamond_block",
        "emerald_block",
        "anvil",
        "chipped_anvil",
        "damaged_anvil",
    ] {
        m.insert(block, 2u8);
    }

    // Level 3: needs diamond+ (obsidian, ancient debris). A diamond pickaxe
    // is sufficient to mine and drop ancient debris in vanilla Minecraft.
    m.insert("obsidian", 3u8);
    m.insert("ancient_debris", 3u8);

    // Level 4: needs netherite (netherite block only).
    m.insert("netherite_block", 4u8);

    // Bedrock is unbreakable regardless of tool.
    m.insert("bedrock", u8::MAX);

    m
});

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
                "sword" => Some(ToolType::Sword),
                "hoe" => Some(ToolType::Hoe),
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

/// Finds the best available tool of the given type in an inventory.
///
/// Returns the slot index of the best tool (highest material priority), or
/// `None` if no matching tool is found.
#[deprecated(note = "use tool_select::find_tool_in_inventory instead")]
pub fn find_best_tool_in_inventory(
    tool_type: &ToolType,
    inventory: &[Option<ItemStack>],
) -> Option<u8> {
    crate::tool_select::find_tool_in_inventory(tool_type, inventory, None).map(|(_, slot)| slot)
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
        assert_eq!(best_tool_for_block("glass"), ToolType::Hand);
        assert_eq!(best_tool_for_block("glass_pane"), ToolType::Hand);
    }

    #[test]
    fn test_best_tool_for_block_unknown() {
        assert_eq!(best_tool_for_block("unknown_block"), ToolType::Hand);
        assert_eq!(best_tool_for_block("not_a_block"), ToolType::Hand);
    }

    #[test]
    fn test_best_tool_for_block_sword_and_hoe() {
        // Sword is the fastest tool for cobweb in Java Edition.
        assert_eq!(best_tool_for_block("cobweb"), ToolType::Sword);
        // Hoe is the fastest tool for these blocks.
        assert_eq!(best_tool_for_block("hay_bale"), ToolType::Hoe);
        assert_eq!(best_tool_for_block("sculk"), ToolType::Hoe);
        assert_eq!(best_tool_for_block("moss_block"), ToolType::Hoe);
        // Leaves variants are mapped to Shears (for drops), not Hoe.
        assert_eq!(best_tool_for_block("oak_leaves"), ToolType::Shears);
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
        assert_eq!(material_from_item_name("stone"), None);
        assert_eq!(material_from_item_name("diamond_hoe_altar"), None);
    }

    #[test]
    fn test_material_from_item_sword() {
        assert_eq!(
            material_from_item_name("diamond_sword"),
            Some((ToolType::Sword, MaterialTier::Diamond))
        );
        assert_eq!(
            material_from_item_name("iron_sword"),
            Some((ToolType::Sword, MaterialTier::Iron))
        );
        assert_eq!(
            material_from_item_name("wooden_sword"),
            Some((ToolType::Sword, MaterialTier::Wood))
        );
    }

    #[test]
    fn test_material_from_item_hoe() {
        assert_eq!(
            material_from_item_name("wooden_hoe"),
            Some((ToolType::Hoe, MaterialTier::Wood))
        );
        assert_eq!(
            material_from_item_name("netherite_hoe"),
            Some((ToolType::Hoe, MaterialTier::Netherite))
        );
        assert_eq!(
            material_from_item_name("diamond_hoe"),
            Some((ToolType::Hoe, MaterialTier::Diamond))
        );
    }

    // --- calculate_mine_time ---

    // --- calculate_mine_time tests ---
    //
    // The dead `block_data::calculate_mine_time` was deleted in Phase 4; the
    // canonical implementation lives in `mining_calc` (with the 1.5× factor)
    // and its tests are in `mining_calc.rs`.

    // --- find_best_tool_in_inventory ---

    #[test]
    fn test_find_best_tool_empty_inventory() {
        let inv: Vec<Option<ItemStack>> = vec![];
        assert_eq!(
            crate::tool_select::find_tool_in_inventory(&ToolType::Pickaxe, &inv, None)
                .map(|(_, slot)| slot),
            None
        );
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
        assert_eq!(
            crate::tool_select::find_tool_in_inventory(&ToolType::Pickaxe, &inv, None)
                .map(|(_, slot)| slot),
            None
        );
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
            crate::tool_select::find_tool_in_inventory(&ToolType::Pickaxe, &inv, None)
                .map(|(_, slot)| slot),
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
            crate::tool_select::find_tool_in_inventory(&ToolType::Shovel, &inv, None)
                .map(|(_, slot)| slot),
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
            crate::tool_select::find_tool_in_inventory(&ToolType::Pickaxe, &inv, None)
                .map(|(_, slot)| slot),
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
                .position(|m| matches!(m, MaterialTier::Gold)),
            Some(4)
        );
        assert_eq!(
            MATERIAL_PRIORITY
                .iter()
                .position(|m| matches!(m, MaterialTier::Wood)),
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

    // --- harvest_level_of ---

    #[test]
    fn test_harvest_level_of_matches_vanilla() {
        assert_eq!(harvest_level_of(MaterialTier::Wood), 0);
        assert_eq!(harvest_level_of(MaterialTier::Gold), 1);
        assert_eq!(harvest_level_of(MaterialTier::Stone), 1);
        assert_eq!(harvest_level_of(MaterialTier::Iron), 2);
        assert_eq!(harvest_level_of(MaterialTier::Diamond), 3);
        assert_eq!(harvest_level_of(MaterialTier::Netherite), 4);
    }

    // --- HARVEST_LEVEL table ---

    #[test]
    fn test_harvest_level_known_blocks() {
        // Hand-mineable (no tool required).
        assert_eq!(HARVEST_LEVEL.get("oak_log").copied(), Some(0));
        assert_eq!(HARVEST_LEVEL.get("dirt").copied(), Some(0));
        assert_eq!(HARVEST_LEVEL.get("glass").copied(), Some(0));
        // `stone` and friends: needs stone+ pickaxe. Vanilla MC's
        // `stone` block requires a pickaxe, so we tag it level 1
        // (stone tier) so the tool selector refuses wood pickaxes.
        // This deviates from raw vanilla metadata (which says stone
        // is wood-tier) but is the safer default — wood pickaxes mine
        // stone so slowly that the bot should prefer not to.
        assert_eq!(HARVEST_LEVEL.get("stone").copied(), Some(1));
        // Needs stone+ pickaxe (i.e. harvest level 1).
        assert_eq!(HARVEST_LEVEL.get("cobblestone").copied(), Some(1));
        assert_eq!(HARVEST_LEVEL.get("iron_ore").copied(), Some(1));
        assert_eq!(HARVEST_LEVEL.get("coal_ore").copied(), Some(1));
        assert_eq!(HARVEST_LEVEL.get("lapis_ore").copied(), Some(1));
        // Needs iron+ pickaxe (i.e. harvest level 2).
        assert_eq!(HARVEST_LEVEL.get("gold_ore").copied(), Some(2));
        assert_eq!(HARVEST_LEVEL.get("redstone_ore").copied(), Some(2));
        assert_eq!(
            HARVEST_LEVEL.get("deepslate_redstone_ore").copied(),
            Some(2)
        );
        assert_eq!(HARVEST_LEVEL.get("diamond_ore").copied(), Some(2));
        assert_eq!(HARVEST_LEVEL.get("deepslate_diamond_ore").copied(), Some(2));
        // Needs diamond+.
        assert_eq!(HARVEST_LEVEL.get("obsidian").copied(), Some(3));
        // Ancient debris needs a diamond pickaxe (level 3), not netherite.
        assert_eq!(HARVEST_LEVEL.get("ancient_debris").copied(), Some(3));
        // Needs netherite.
        assert_eq!(HARVEST_LEVEL.get("netherite_block").copied(), Some(4));
        // Unbreakable.
        assert_eq!(HARVEST_LEVEL.get("bedrock").copied(), Some(u8::MAX));
    }

    #[test]
    fn test_harvest_level_unknown_block_defaults_zero() {
        assert_eq!(HARVEST_LEVEL.get("not_a_block").copied(), None);
    }
}
