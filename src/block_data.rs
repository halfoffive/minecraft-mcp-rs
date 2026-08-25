//! Block and tool data tables for the Minecraft MCP server.
//!
//! Provides lookup functions for block-to-tool mappings, material tier speeds,
//! tool name patterns, block hardness values, and mining time calculations.
//!
//! > **Note:** Most items in this module are lookup tables designed for the
//! > bot ops layer.  They are retained for the integration plan.

use std::collections::{HashMap, HashSet};
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
    //
    // Vanilla 1.21.11 `mineable/pickaxe` membership, extracted from the
    // azalea-generated registry data (the ONLY supported Minecraft version —
    // the same data the bot library is generated from). Note that chests
    // are NOT pickaxe blocks in vanilla — they live in the Axe list below
    // (report M-19).
    for &block in &[
        "stone",
        "cobblestone",
        "mossy_cobblestone",
        "andesite",
        "diorite",
        "granite",
        "stone_bricks",
        "mossy_stone_bricks",
        "cracked_stone_bricks",
        "smooth_stone",
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
        "nether_gold_ore",
        // Deepslate family
        "deepslate",
        "cobbled_deepslate",
        "polished_deepslate",
        "deepslate_bricks",
        "deepslate_tiles",
        "cobbled_deepslate_stairs",
        "cobbled_deepslate_slab",
        "cobbled_deepslate_wall",
        "tuff",
        "calcite",
        // Copper family (all require stone+ — see HARVEST_LEVEL)
        "copper_block",
        "exposed_copper",
        "weathered_copper",
        "oxidized_copper",
        "cut_copper",
        "raw_copper_block",
        "raw_iron_block",
        "raw_gold_block",
        // Blackstone / basalt family
        "blackstone",
        "polished_blackstone",
        "polished_blackstone_bricks",
        "gilded_blackstone",
        "blackstone_stairs",
        "blackstone_slab",
        "blackstone_wall",
        "basalt",
        "polished_basalt",
        "smooth_basalt",
        // Nether / End
        "netherrack",
        "nether_quartz_ore",
        "end_stone",
        "end_stone_bricks",
        "purpur_block",
        "purpur_pillar",
        "nether_bricks",
        "red_nether_bricks",
        "nether_brick_stairs",
        "nether_brick_slab",
        "nether_brick_fence",
        "magma_block",
        "bone_block",
        // Sandstone family
        "sandstone",
        "red_sandstone",
        "smooth_sandstone",
        "chiseled_sandstone",
        "cut_sandstone",
        "sandstone_stairs",
        "sandstone_slab",
        "red_sandstone_stairs",
        // Quartz family
        "quartz_block",
        "smooth_quartz",
        "chiseled_quartz_block",
        "quartz_pillar",
        "quartz_bricks",
        "quartz_stairs",
        // Prismarine family
        "prismarine",
        "prismarine_bricks",
        "dark_prismarine",
        "prismarine_stairs",
        "prismarine_slab",
        // Mud brick family
        "mud_bricks",
        "packed_mud",
        "mud_brick_stairs",
        "mud_brick_slab",
        // Terracotta family
        "terracotta",
        "white_terracotta",
        "orange_terracotta",
        "magenta_terracotta",
        "light_blue_terracotta",
        "yellow_terracotta",
        "lime_terracotta",
        "pink_terracotta",
        "gray_terracotta",
        "light_gray_terracotta",
        "cyan_terracotta",
        "purple_terracotta",
        "blue_terracotta",
        "brown_terracotta",
        "green_terracotta",
        "red_terracotta",
        "black_terracotta",
        // Amethyst
        "amethyst_block",
        "budding_amethyst",
        // Dripstone
        "dripstone_block",
        // Ice (pickaxe is the fastest tool; drops need silk touch either
        // way, so harvest level stays 0)
        "ice",
        "packed_ice",
        "blue_ice",
        // Manufactured
        "bricks",
        "brick_slab",
        "brick_stairs",
        "iron_block",
        "gold_block",
        "diamond_block",
        "emerald_block",
        "lapis_block",
        "coal_block",
        "redstone_block",
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
        "ender_chest",
        // Netherite blocks
        "ancient_debris",
        "netherite_block",
    ] {
        m.insert(block, ToolType::Pickaxe);
    }

    // --- Axe blocks ---
    //
    // Vanilla 1.21.11 `mineable/axe` membership. Chests are axe blocks in
    // vanilla (report M-19: they were misfiled under Pickaxe, and with an
    // over-strict harvest requirement on top — they are now level 0, so a
    // bot without any axe can still break them by hand, exactly like
    // vanilla).
    for &block in &[
        "chest",
        "trapped_chest",
        "oak_log",
        "spruce_log",
        "birch_log",
        "jungle_log",
        "acacia_log",
        "dark_oak_log",
        "mangrove_log",
        "cherry_log",
        "bamboo_block",
        "crimson_stem",
        "warped_stem",
        "oak_planks",
        "spruce_planks",
        "birch_planks",
        "jungle_planks",
        "acacia_planks",
        "dark_oak_planks",
        "mangrove_planks",
        "cherry_planks",
        "bamboo_planks",
        "bamboo_mosaic",
        "crimson_planks",
        "warped_planks",
        "oak_stairs",
        "spruce_stairs",
        "birch_stairs",
        "mangrove_stairs",
        "cherry_stairs",
        "bamboo_stairs",
        "crimson_stairs",
        "warped_stairs",
        "oak_slab",
        "spruce_slab",
        "birch_slab",
        "mangrove_slab",
        "cherry_slab",
        "bamboo_slab",
        "crimson_slab",
        "warped_slab",
        "oak_fence",
        "spruce_fence",
        "birch_fence",
        "mangrove_fence",
        "cherry_fence",
        "bamboo_fence",
        "crimson_fence",
        "warped_fence",
        "oak_fence_gate",
        "mangrove_fence_gate",
        "cherry_fence_gate",
        "bamboo_fence_gate",
        "crimson_fence_gate",
        "warped_fence_gate",
        "oak_door",
        "spruce_door",
        "birch_door",
        "mangrove_door",
        "cherry_door",
        "bamboo_door",
        "crimson_door",
        "warped_door",
        "oak_trapdoor",
        "spruce_trapdoor",
        "birch_trapdoor",
        "jungle_trapdoor",
        "acacia_trapdoor",
        "dark_oak_trapdoor",
        "mangrove_trapdoor",
        "cherry_trapdoor",
        "bamboo_trapdoor",
        "crimson_trapdoor",
        "warped_trapdoor",
        "jungle_stairs",
        "acacia_stairs",
        "dark_oak_stairs",
        "jungle_slab",
        "acacia_slab",
        "dark_oak_slab",
        "jungle_fence",
        "acacia_fence",
        "dark_oak_fence",
        "jungle_fence_gate",
        "acacia_fence_gate",
        "dark_oak_fence_gate",
        "jungle_door",
        "acacia_door",
        "dark_oak_door",
        "crafting_table",
        "bookshelf",
        "ladder",
        "barrel",
        // Gourd / hive blocks are axe-mineable in vanilla
        "pumpkin",
        "melon",
        "bee_nest",
        "beehive",
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
        "mangrove_leaves",
        "cherry_leaves",
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
    // Hoes are the fastest tool for hay blocks, sculk, moss, sponges, and
    // shroomlight (vanilla 1.21.11 `mineable/hoe`). NOTE: the block id is
    // `hay_block` — the previous `hay_bale` entry never matched a real
    // snapshot block name. (Leaves variants are already mapped to Shears
    // above.)
    for &block in &[
        "hay_block",
        "sculk",
        "moss_block",
        "sponge",
        "wet_sponge",
        "shroomlight",
    ] {
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
/// These are vanilla *hardness* (`strength`) values, NOT fist-mining
/// seconds — [`calculate_mine_time`](crate::mining_calc::calculate_mine_time)
/// converts them into break times (e.g. stone at 1.5 takes 7.5 s by hand).
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

    // Nether / End
    m.insert("netherrack", 0.4);
    m.insert("nether_quartz_ore", 3.0);
    m.insert("nether_gold_ore", 3.0);
    m.insert("end_stone", 3.0);
    m.insert("end_stone_bricks", 3.0);
    // Vanilla 1.21.11 hardness is 1.5 for both (the old 3.0 doubled the
    // wait — safe direction, but inaccurate).
    m.insert("purpur_block", 1.5);
    m.insert("purpur_pillar", 1.5);

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
    // Vanilla 1.21.11 hardness is 0.25 (report M-20: the old 0.1 halved the
    // mining wait below the real break time).
    m.insert("powder_snow", 0.25);
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
    // Vanilla 1.21.11 hardness is 2.8 (report M-20: the old 0.5 made the
    // mining wait 5.6× too short, so every blue-ice dig timed out as
    // MiningInterrupted).
    m.insert("blue_ice", 2.8);
    m.insert("snow", 0.1);
    m.insert("hay_block", 0.5);
    m.insert("cobweb", 4.0);
    m.insert("pumpkin", 1.0);
    m.insert("melon", 1.0);
    m.insert("bee_nest", 0.3);
    m.insert("beehive", 0.6);
    m.insert("sponge", 0.6);
    m.insert("wet_sponge", 0.6);
    m.insert("shroomlight", 1.0);
    m.insert("sea_lantern", 0.3);
    m.insert("glowstone", 0.3);
    m.insert("magma_block", 0.5);
    m.insert("bone_block", 2.0);
    m.insert("dripstone_block", 1.5);
    m.insert("pointed_dripstone", 1.5);

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
    // Vanilla 1.21.11 hardness is 3.0 (report M-20: the old 5.0 over-
    // estimated — the safe direction, but inaccurate).
    m.insert("gold_block", 3.0);
    m.insert("diamond_block", 5.0);
    m.insert("emerald_block", 5.0);
    m.insert("lapis_block", 3.0);
    m.insert("coal_block", 5.0);
    m.insert("redstone_block", 5.0);
    m.insert("raw_iron_block", 5.0);
    m.insert("raw_copper_block", 5.0);
    m.insert("raw_gold_block", 5.0);
    m.insert("netherite_block", 50.0);

    // Ancient debris (needs diamond+ pickaxe, drops as raw ancient debris)
    m.insert("ancient_debris", 30.0);

    // ── Coverage additions (report M-22) ─────────────────────
    // Values are the vanilla 1.21.11 `strength` extracted from the
    // azalea-generated block data.
    //
    // Wood families added in the 2026-08 review round (mangrove, cherry,
    // bamboo, crimson, warped): logs/stems/planks/mosaic 2.0, doors and
    // trapdoors 3.0, everything else 2.0. Leaves 0.2 like the other trees.
    m.insert("mangrove_log", 2.0);
    m.insert("cherry_log", 2.0);
    m.insert("bamboo_block", 2.0);
    m.insert("crimson_stem", 2.0);
    m.insert("warped_stem", 2.0);
    for &block in &[
        "mangrove_planks",
        "cherry_planks",
        "bamboo_planks",
        "bamboo_mosaic",
        "crimson_planks",
        "warped_planks",
        "mangrove_stairs",
        "cherry_stairs",
        "bamboo_stairs",
        "crimson_stairs",
        "warped_stairs",
        "mangrove_slab",
        "cherry_slab",
        "bamboo_slab",
        "crimson_slab",
        "warped_slab",
        "mangrove_fence",
        "cherry_fence",
        "bamboo_fence",
        "crimson_fence",
        "warped_fence",
        "mangrove_fence_gate",
        "cherry_fence_gate",
        "bamboo_fence_gate",
        "crimson_fence_gate",
        "warped_fence_gate",
        "jungle_stairs",
        "acacia_stairs",
        "dark_oak_stairs",
        "jungle_slab",
        "acacia_slab",
        "dark_oak_slab",
        "jungle_fence",
        "acacia_fence",
        "dark_oak_fence",
        "jungle_fence_gate",
        "acacia_fence_gate",
        "dark_oak_fence_gate",
    ] {
        m.insert(block, 2.0);
    }
    for &block in &[
        "mangrove_door",
        "cherry_door",
        "bamboo_door",
        "crimson_door",
        "warped_door",
        "mangrove_trapdoor",
        "cherry_trapdoor",
        "bamboo_trapdoor",
        "crimson_trapdoor",
        "warped_trapdoor",
        "oak_trapdoor",
        "spruce_trapdoor",
        "birch_trapdoor",
        "jungle_trapdoor",
        "acacia_trapdoor",
        "dark_oak_trapdoor",
        "jungle_door",
        "acacia_door",
        "dark_oak_door",
    ] {
        m.insert(block, 3.0);
    }
    m.insert("mangrove_leaves", 0.2);
    m.insert("cherry_leaves", 0.2);

    // Copper family (all stone+ tier; see HARVEST_LEVEL).
    m.insert("copper_block", 3.0);
    m.insert("exposed_copper", 3.0);
    m.insert("weathered_copper", 3.0);
    m.insert("oxidized_copper", 3.0);
    m.insert("cut_copper", 3.0);
    m.insert("exposed_cut_copper", 3.0);
    m.insert("waxed_copper_block", 3.0);

    // Deepslate building family (stone itself stays 3.0 above).
    m.insert("cobbled_deepslate", 3.5);
    m.insert("polished_deepslate", 3.5);
    m.insert("deepslate_bricks", 3.5);
    m.insert("deepslate_tiles", 3.5);
    m.insert("cobbled_deepslate_stairs", 3.5);
    m.insert("cobbled_deepslate_slab", 3.5);
    m.insert("cobbled_deepslate_wall", 3.5);

    // Blackstone / basalt family.
    m.insert("blackstone", 1.5);
    m.insert("polished_blackstone", 2.0);
    m.insert("polished_blackstone_bricks", 1.5);
    m.insert("gilded_blackstone", 1.5);
    m.insert("blackstone_stairs", 1.5);
    m.insert("blackstone_slab", 2.0);
    m.insert("blackstone_wall", 1.5);
    m.insert("basalt", 1.25);
    m.insert("polished_basalt", 1.25);
    m.insert("smooth_basalt", 1.25);

    // Sandstone family (smooth variants are 2.0, the rest 0.8).
    m.insert("sandstone", 0.8);
    m.insert("red_sandstone", 0.8);
    m.insert("chiseled_sandstone", 0.8);
    m.insert("cut_sandstone", 0.8);
    m.insert("sandstone_stairs", 0.8);
    m.insert("red_sandstone_stairs", 0.8);
    m.insert("sandstone_slab", 2.0);
    m.insert("smooth_sandstone", 2.0);
    m.insert("smooth_sandstone_stairs", 2.0);

    // Quartz family (smooth quartz 2.0, the rest 0.8).
    m.insert("quartz_block", 0.8);
    m.insert("chiseled_quartz_block", 0.8);
    m.insert("quartz_pillar", 0.8);
    m.insert("quartz_bricks", 0.8);
    m.insert("quartz_stairs", 0.8);
    m.insert("smooth_quartz", 2.0);

    // Prismarine, nether brick, mud brick families.
    m.insert("prismarine", 1.5);
    m.insert("prismarine_bricks", 1.5);
    m.insert("dark_prismarine", 1.5);
    m.insert("prismarine_stairs", 1.5);
    m.insert("prismarine_slab", 1.5);
    m.insert("nether_bricks", 2.0);
    m.insert("red_nether_bricks", 2.0);
    m.insert("nether_brick_stairs", 2.0);
    m.insert("nether_brick_slab", 2.0);
    m.insert("nether_brick_fence", 2.0);
    m.insert("mud_bricks", 1.5);
    m.insert("packed_mud", 1.0);
    m.insert("mud_brick_stairs", 1.5);
    m.insert("mud_brick_slab", 1.5);

    // Terracotta family (uniform 1.25).
    m.insert("terracotta", 1.25);
    for &block in &[
        "white_terracotta",
        "orange_terracotta",
        "magenta_terracotta",
        "light_blue_terracotta",
        "yellow_terracotta",
        "lime_terracotta",
        "pink_terracotta",
        "gray_terracotta",
        "light_gray_terracotta",
        "cyan_terracotta",
        "purple_terracotta",
        "blue_terracotta",
        "brown_terracotta",
        "green_terracotta",
        "red_terracotta",
        "black_terracotta",
    ] {
        m.insert(block, 1.25);
    }

    // Amethyst.
    m.insert("amethyst_block", 1.5);
    m.insert("budding_amethyst", 1.5);

    // Common stone-family blocks missing from the original table.
    m.insert("mossy_cobblestone", 2.0);
    m.insert("smooth_stone", 2.0);

    m
});

/// Material tier priority order — from best (index 0) to worst (index N).
///
/// Used by [`crate::tool_select::find_tool_in_inventory`] to select the
/// highest-tier tool. This is the reverse of the `Ord` derive on
/// [`MaterialTier`] (whose variant order is
/// `Wood < Gold < Stone < Iron < Diamond < Netherite`), so the highest-`Ord`
/// tier is preferred. Gold ranks above Wood (it has the same mining level but
/// higher speed), and below Stone (lower durability and mining level).
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
        "packed_mud",
        "muddy_mangrove_roots",
        "mycelium",
        "podzol",
        "pumpkin",
        "melon",
        "bee_nest",
        "beehive",
        "sponge",
        "wet_sponge",
        "oak_leaves",
        "spruce_leaves",
        "birch_leaves",
        "jungle_leaves",
        "acacia_leaves",
        "dark_oak_leaves",
        "azalea_leaves",
        "mangrove_leaves",
        "cherry_leaves",
        // All 16 wool colours are level 0 (the original table listed only
        // white_wool; the others silently defaulted to 0 anyway — this just
        // makes the coverage explicit).
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
        "cobweb",
        "hay_block",
        "sculk",
        "moss_block",
        "shroomlight",
        "vine",
        "glow_lichen",
        "glass",
        "glass_pane",
        "white_stained_glass",
        "white_stained_glass_pane",
        "ice",
        "packed_ice",
        "blue_ice",
        // Chests are level 0 in vanilla — any tool (or hand) breaks them
        // and they drop with their contents (report M-19: the old table
        // demanded a stone+ PICKAXE and refused the bare-hand path).
        "chest",
        "trapped_chest",
        "ender_chest",
        // Nether gold ore is level 0 in vanilla 1.21.11 — it appears in NO
        // `needs_*_tool` / `incorrect_for_*_tool` tag (verified against the
        // azalea-generated tag data). The 2026-08 audit's "level 2" claim
        // was wrong; the stone-tier over-requirement let a bot skip a legal
        // wooden/stone pick (report M-21 adjudication).
        "nether_gold_ore",
        // Wood families added in the 2026-08 review round.
        "mangrove_log",
        "mangrove_planks",
        "cherry_log",
        "cherry_planks",
        "bamboo_block",
        "bamboo_planks",
        "bamboo_mosaic",
        "crimson_stem",
        "crimson_planks",
        "warped_stem",
        "warped_planks",
        // Stone-family building blocks with NO vanilla tier requirement
        // (they DO require the correct tool for drops — see
        // TOOL_REQUIRED_FOR_DROPS).
        "cobbled_deepslate",
        "polished_deepslate",
        "deepslate_bricks",
        "deepslate_tiles",
        "blackstone",
        "polished_blackstone",
        "polished_blackstone_bricks",
        "gilded_blackstone",
        "basalt",
        "polished_basalt",
        "smooth_basalt",
        "sandstone",
        "red_sandstone",
        "smooth_sandstone",
        "quartz_block",
        "smooth_quartz",
        "prismarine",
        "prismarine_bricks",
        "dark_prismarine",
        "nether_bricks",
        "red_nether_bricks",
        "mud_bricks",
        "terracotta",
        "white_terracotta",
        "orange_terracotta",
        "magenta_terracotta",
        "light_blue_terracotta",
        "yellow_terracotta",
        "lime_terracotta",
        "pink_terracotta",
        "gray_terracotta",
        "light_gray_terracotta",
        "cyan_terracotta",
        "purple_terracotta",
        "blue_terracotta",
        "brown_terracotta",
        "green_terracotta",
        "red_terracotta",
        "black_terracotta",
        "amethyst_block",
        "budding_amethyst",
        "dripstone_block",
        "mossy_cobblestone",
        "smooth_stone",
        "end_stone_bricks",
        "bone_block",
        "magma_block",
        "coal_block",
        "redstone_block",
        "lapis_block",
    ] {
        m.insert(block, 0u8);
    }

    // Level 1: needs stone+ (vanilla needs_stone_tool — iron/copper/lapis
    // ores, copper blocks, and the raw copper/iron blocks).
    //
    // The first ten entries are a DOCUMENTED CONSERVATIVE DEVIATION: vanilla
    // lists them at level 0 (any pickaxe drops them), but the bot refuses a
    // wooden pickaxe for them because wood mining is so slow the bot should
    // prefer to wait for stone+. test_harvest_level_known_blocks pins
    // stone=1 and cobblestone=1; the deviation was deliberately introduced
    // in the 2026-08 audit round and is kept. Everything else the old table
    // carried at level 1 (andesite, stone bricks, furnace, hopper, chests,
    // ...) moved to level 0 in the 2026-08 review round: vanilla has no
    // tier requirement for those blocks, and the over-requirement made the
    // bot refuse legal mining (report M-19/M-22).
    for &block in &[
        // Documented conservative over-requirements (vanilla level 0):
        "stone",
        "cobblestone",
        "coal_ore",
        "deepslate_coal_ore",
        "netherrack",
        "nether_quartz_ore",
        "end_stone",
        "purpur_block",
        "purpur_pillar",
        "deepslate",
        // Vanilla level 1 (needs_stone_tool):
        "iron_ore",
        "deepslate_iron_ore",
        "copper_ore",
        "deepslate_copper_ore",
        "lapis_ore",
        "deepslate_lapis_ore",
        "copper_block",
        "exposed_copper",
        "weathered_copper",
        "oxidized_copper",
        "cut_copper",
        "raw_copper_block",
        "raw_iron_block",
        "iron_block",
    ] {
        m.insert(block, 1u8);
    }

    // Level 2: needs iron+ (vanilla needs_iron_tool: gold/redstone/diamond/
    // emerald ores, their deepslate variants, and the gold/diamond/emerald/
    // raw-gold blocks).
    //
    // NOTE: nether_gold_ore was moved OUT of this list in the 2026-08 review
    // round — the audit's "iron+" claim was refuted by the vanilla 1.21.11
    // tag data (it appears in no needs_*/incorrect_for_* tag; see its entry
    // in the level-0 list). iron_block moved out as well — vanilla
    // needs_stone_tool lists it at level 1. anvil/chipped_anvil/
    // damaged_anvil have no vanilla tier requirement at all (level 0 +
    // correct tool required for drops).
    for &block in &[
        "gold_ore",
        "deepslate_gold_ore",
        "redstone_ore",
        "deepslate_redstone_ore",
        "diamond_ore",
        "deepslate_diamond_ore",
        "emerald_ore",
        "deepslate_emerald_ore",
        "gold_block",
        "diamond_block",
        "emerald_block",
        "raw_gold_block",
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

/// Blocks whose drops REQUIRE the correct tool even though they have no
/// harvest-tier (level) requirement.
///
/// Extracted from vanilla 1.21.11's requires_correct_tool_for_drops block
/// property (via the azalea-generated block behaviours), intersected with
/// the blocks this crate knows about (BLOCK_TO_TOOL_TYPE). Mining one of
/// these WITHOUT its correct tool breaks the block but drops nothing — e.g.
/// cobbled_deepslate by hand. This is the second half of vanilla's mining
/// gate next to HARVEST_LEVEL: tier 0 does NOT mean "hand is fine" for
/// these blocks.
pub static TOOL_REQUIRED_FOR_DROPS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    let blocks: &[&str] = &[
        // Stone family
        "stone",
        "cobblestone",
        "mossy_cobblestone",
        "andesite",
        "diorite",
        "granite",
        "stone_bricks",
        "mossy_stone_bricks",
        "cracked_stone_bricks",
        "smooth_stone",
        "stone_slab",
        "cobblestone_slab",
        "stone_stairs",
        "cobblestone_stairs",
        "cobblestone_wall",
        // Ores (every pickaxe ore, regardless of tier)
        "coal_ore",
        "iron_ore",
        "gold_ore",
        "diamond_ore",
        "emerald_ore",
        "lapis_ore",
        "redstone_ore",
        "copper_ore",
        "nether_gold_ore",
        "deepslate_coal_ore",
        "deepslate_iron_ore",
        "deepslate_gold_ore",
        "deepslate_diamond_ore",
        "deepslate_emerald_ore",
        "deepslate_lapis_ore",
        "deepslate_redstone_ore",
        "deepslate_copper_ore",
        // Deepslate building family
        "deepslate",
        "cobbled_deepslate",
        "polished_deepslate",
        "deepslate_bricks",
        "deepslate_tiles",
        "cobbled_deepslate_stairs",
        "cobbled_deepslate_slab",
        "cobbled_deepslate_wall",
        "tuff",
        "calcite",
        // Copper family + raw metal blocks
        "copper_block",
        "exposed_copper",
        "weathered_copper",
        "oxidized_copper",
        "cut_copper",
        "raw_copper_block",
        "raw_iron_block",
        "raw_gold_block",
        // Blackstone / basalt family
        "blackstone",
        "polished_blackstone",
        "polished_blackstone_bricks",
        "gilded_blackstone",
        "blackstone_stairs",
        "blackstone_slab",
        "blackstone_wall",
        "basalt",
        "polished_basalt",
        "smooth_basalt",
        // Nether / End
        "netherrack",
        "nether_quartz_ore",
        "end_stone",
        "end_stone_bricks",
        "purpur_block",
        "purpur_pillar",
        "nether_bricks",
        "red_nether_bricks",
        "nether_brick_stairs",
        "nether_brick_slab",
        "nether_brick_fence",
        "magma_block",
        "bone_block",
        // Sandstone family
        "sandstone",
        "red_sandstone",
        "smooth_sandstone",
        "chiseled_sandstone",
        "cut_sandstone",
        "sandstone_stairs",
        "sandstone_slab",
        "red_sandstone_stairs",
        // Quartz family
        "quartz_block",
        "smooth_quartz",
        "chiseled_quartz_block",
        "quartz_pillar",
        "quartz_bricks",
        "quartz_stairs",
        // Prismarine family
        "prismarine",
        "prismarine_bricks",
        "dark_prismarine",
        "prismarine_stairs",
        "prismarine_slab",
        // Mud brick family
        "mud_bricks",
        "packed_mud",
        "mud_brick_stairs",
        "mud_brick_slab",
        // Terracotta family
        "terracotta",
        "white_terracotta",
        "orange_terracotta",
        "magenta_terracotta",
        "light_blue_terracotta",
        "yellow_terracotta",
        "lime_terracotta",
        "pink_terracotta",
        "gray_terracotta",
        "light_gray_terracotta",
        "cyan_terracotta",
        "purple_terracotta",
        "blue_terracotta",
        "brown_terracotta",
        "green_terracotta",
        "red_terracotta",
        "black_terracotta",
        // Amethyst / dripstone
        "amethyst_block",
        "budding_amethyst",
        "dripstone_block",
        // Bricks
        "bricks",
        "brick_slab",
        "brick_stairs",
        // Metal / mineral blocks
        "iron_block",
        "gold_block",
        "diamond_block",
        "emerald_block",
        "lapis_block",
        "coal_block",
        "redstone_block",
        // Machinery (all drop only with a pickaxe)
        "furnace",
        "blast_furnace",
        "smoker",
        "anvil",
        "chipped_anvil",
        "damaged_anvil",
        "enchanting_table",
        "hopper",
        "dropper",
        "dispenser",
        "observer",
        // Hardest materials
        "obsidian",
        "ancient_debris",
        "netherite_block",
        // Non-pickaxe tools: cobweb needs sword/shears for string, snow
        // needs a shovel for snowballs.
        "cobweb",
        "snow",
        "snow_block",
    ];
    blocks.iter().copied().collect()
});

/// Whether the block drops nothing unless mined with its correct tool,
/// independent of any harvest-tier requirement.
///
/// Vanilla's requires_correct_tool_for_drops property (see
/// TOOL_REQUIRED_FOR_DROPS). Combined with HARVEST_LEVEL this forms the
/// full vanilla mining gate: refusing to mine a tier-0 block by hand is
/// only correct when this returns true (report M-16 — dirt, sand, logs,
/// wool etc. are legal hand-mines and must not be refused).
pub fn requires_tool_for_drops(block_type: &str) -> bool {
    TOOL_REQUIRED_FOR_DROPS.contains(block_type)
}

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
        assert_eq!(best_tool_for_block("hay_block"), ToolType::Hoe);
        assert_eq!(best_tool_for_block("sculk"), ToolType::Hoe);
        assert_eq!(best_tool_for_block("moss_block"), ToolType::Hoe);
        assert_eq!(best_tool_for_block("sponge"), ToolType::Hoe);
        assert_eq!(best_tool_for_block("shroomlight"), ToolType::Hoe);
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

    // --- tool_select::find_tool_in_inventory (delegation target of the
    // deleted find_best_tool_in_inventory shim, audit L-25) ---

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

    // ── minimum_material_for_harvest_level (audit L-33 coverage) ──

    #[test]
    fn test_minimum_material_for_harvest_level_maps_each_level() {
        // Level 0 → Wood (anything works; Wood is the weakest known tier).
        assert_eq!(
            minimum_material_for_harvest_level(0),
            Some(MaterialTier::Wood)
        );
        // Level 1 → Stone (covers Gold, which shares Stone's level 1).
        assert_eq!(
            minimum_material_for_harvest_level(1),
            Some(MaterialTier::Stone)
        );
        assert_eq!(
            minimum_material_for_harvest_level(2),
            Some(MaterialTier::Iron)
        );
        assert_eq!(
            minimum_material_for_harvest_level(3),
            Some(MaterialTier::Diamond)
        );
        assert_eq!(
            minimum_material_for_harvest_level(4),
            Some(MaterialTier::Netherite)
        );
    }

    #[test]
    fn test_minimum_material_for_harvest_level_out_of_range() {
        // Levels above the highest known tier (Netherite = 4) have no
        // material that meets them.
        assert_eq!(minimum_material_for_harvest_level(5), None);
        assert_eq!(minimum_material_for_harvest_level(255), None);
    }

    #[test]
    fn test_minimum_material_matches_harvest_level_roundtrip() {
        // A material returned for a level must itself have exactly that
        // harvest level (the mapping is the inverse of harvest_level_of).
        for level in 0..=4u8 {
            let mat = minimum_material_for_harvest_level(level).unwrap();
            assert_eq!(
                harvest_level_of(mat),
                level,
                "minimum material for level {level} must itself be level {level}"
            );
        }
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

    // ── M-21 (report): nether_gold_ore adjudicated to level 0 ───────

    /// Report M-21 adjudication: the vanilla 1.21.11 data (extracted from
    /// the azalea-generated block tags) lists nether_gold_ore in NO
    /// needs_*_tool / incorrect_for_*_tool tag — ANY pickaxe drops it. The
    /// 2026-08 audit's "level 2 (iron+)" correction was wrong; the table
    /// now carries level 0 so wooden and stone pickaxes are accepted
    /// (select_tool_for_block applies no tier filter for level-0 blocks).
    ///
    /// Conservative deviations from vanilla KEPT at level 1 (documented in
    /// the HARVEST_LEVEL table): stone, cobblestone, coal_ore,
    /// deepslate_coal_ore, netherrack, end_stone, purpur_block/
    /// purpur_pillar, deepslate, nether_quartz_ore.
    #[test]
    fn test_harvest_level_nether_gold_ore_is_level_zero() {
        assert_eq!(
            HARVEST_LEVEL.get("nether_gold_ore").copied(),
            Some(0),
            "nether_gold_ore has no vanilla tier requirement (any pickaxe drops it)"
        );
        // With no tier filter, even a wooden pickaxe is selectable.
        let wood_inv = vec![Some(ItemStack {
            item_id: "wooden_pickaxe".to_string(),
            count: 1,
        })];
        assert_eq!(
            crate::tool_select::find_tool_in_inventory(&ToolType::Pickaxe, &wood_inv, Some(0)),
            Some((MaterialTier::Wood, 0)),
            "wooden pickaxe must be accepted for nether_gold_ore"
        );
    }

    // ── M-19 (report): chests are axe blocks with no tier gate ──────

    /// Report M-19: vanilla chests are mineable/axe with NO harvest
    /// requirement. The old table listed them as Pickaxe at level 1, so a
    /// bot without a stone+ pickaxe got ToolNotFound even though vanilla
    /// lets you break a chest bare-handed.
    #[test]
    fn test_chest_is_axe_block_without_harvest_gate() {
        assert_eq!(best_tool_for_block("chest"), ToolType::Axe);
        assert_eq!(best_tool_for_block("trapped_chest"), ToolType::Axe);
        assert_eq!(best_tool_for_block("ender_chest"), ToolType::Pickaxe);
        assert_eq!(HARVEST_LEVEL.get("chest").copied(), Some(0));
        assert_eq!(HARVEST_LEVEL.get("trapped_chest").copied(), Some(0));
        assert_eq!(HARVEST_LEVEL.get("ender_chest").copied(), Some(0));
        // None of the chests requires the correct tool for drops — a hand
        // break still yields the chest (contents included).
        assert!(!requires_tool_for_drops("chest"));
        assert!(!requires_tool_for_drops("trapped_chest"));
        assert!(!requires_tool_for_drops("ender_chest"));
    }

    // ── M-20 (report): hardness corrections ─────────────────────────

    /// Report M-20: blue_ice 0.5→2.8 (mining wait was 5.6x too short),
    /// gold_block 5.0→3.0, powder_snow 0.1→0.25, purpur 3.0→1.5.
    #[test]
    fn test_hardness_corrections_match_vanilla() {
        assert_eq!(BLOCK_HARDNESS.get("blue_ice").copied(), Some(2.8));
        assert_eq!(BLOCK_HARDNESS.get("gold_block").copied(), Some(3.0));
        assert_eq!(BLOCK_HARDNESS.get("powder_snow").copied(), Some(0.25));
        assert_eq!(BLOCK_HARDNESS.get("purpur_block").copied(), Some(1.5));
        assert_eq!(BLOCK_HARDNESS.get("purpur_pillar").copied(), Some(1.5));
        // Unchanged anchors so the corrections cannot silently regress
        // neighbouring entries.
        assert_eq!(BLOCK_HARDNESS.get("packed_ice").copied(), Some(0.5));
        assert_eq!(BLOCK_HARDNESS.get("ice").copied(), Some(0.5));
        assert_eq!(BLOCK_HARDNESS.get("iron_block").copied(), Some(5.0));
    }

    // ── M-22 (report): coverage additions spot-checks ───────────────

    /// Report M-22: previously-unregistered block families must now map
    /// to the vanilla tool and tier instead of silently falling back to
    /// "hand, no gate".
    #[test]
    fn test_coverage_additions_match_vanilla() {
        // New wood families are axe blocks, level 0, no drop gate.
        for block in [
            "mangrove_planks",
            "cherry_log",
            "bamboo_planks",
            "crimson_stem",
            "warped_planks",
        ] {
            assert_eq!(best_tool_for_block(block), ToolType::Axe, "{block}");
            assert_eq!(HARVEST_LEVEL.get(block).copied(), Some(0), "{block}");
            assert!(!requires_tool_for_drops(block), "{block}");
        }
        // Copper needs stone+ (vanilla needs_stone_tool).
        assert_eq!(best_tool_for_block("copper_block"), ToolType::Pickaxe);
        assert_eq!(HARVEST_LEVEL.get("copper_block").copied(), Some(1));
        assert_eq!(HARVEST_LEVEL.get("raw_copper_block").copied(), Some(1));
        assert_eq!(HARVEST_LEVEL.get("raw_iron_block").copied(), Some(1));
        assert_eq!(HARVEST_LEVEL.get("raw_gold_block").copied(), Some(2));
        // Cobbled deepslate: pickaxe, level 0, but hand drops nothing.
        assert_eq!(best_tool_for_block("cobbled_deepslate"), ToolType::Pickaxe);
        assert_eq!(HARVEST_LEVEL.get("cobbled_deepslate").copied(), Some(0));
        assert!(requires_tool_for_drops("cobbled_deepslate"));
        // Blackstone / basalt / sandstone / quartz / prismarine /
        // nether brick / mud brick / terracotta / amethyst families.
        for block in [
            "blackstone",
            "basalt",
            "sandstone",
            "quartz_block",
            "prismarine",
            "nether_bricks",
            "mud_bricks",
            "terracotta",
            "white_terracotta",
            "amethyst_block",
            "dripstone_block",
        ] {
            assert_eq!(best_tool_for_block(block), ToolType::Pickaxe, "{block}");
            assert_eq!(HARVEST_LEVEL.get(block).copied(), Some(0), "{block}");
            assert!(requires_tool_for_drops(block), "{block}");
        }
        // Anvil family: no vanilla tier requirement (moved from level 2),
        // but a pickaxe is still required for drops.
        assert_eq!(HARVEST_LEVEL.get("anvil"), None);
        assert!(requires_tool_for_drops("anvil"));
        // iron_block is level 1 in vanilla (was level 2).
        assert_eq!(HARVEST_LEVEL.get("iron_block").copied(), Some(1));
    }

    // ── TOOL_REQUIRED_FOR_DROPS ─────────────────────────────────────

    /// The drop gate must cover the tool-required families and stay out
    /// of the legal hand-mining blocks (report M-16).
    #[test]
    fn test_requires_tool_for_drops_split() {
        // Requires the correct tool for drops.
        assert!(requires_tool_for_drops("stone"));
        assert!(requires_tool_for_drops("furnace"));
        assert!(requires_tool_for_drops("hopper"));
        assert!(requires_tool_for_drops("cobweb"));
        assert!(requires_tool_for_drops("snow_block"));
        // Legal hand mines — must NOT be gated.
        for block in [
            "dirt",
            "sand",
            "gravel",
            "oak_log",
            "oak_planks",
            "chest",
            "white_wool",
            "oak_leaves",
            "pumpkin",
            "melon",
            "sponge",
            "ice",
            "glass",
        ] {
            assert!(
                !requires_tool_for_drops(block),
                "{block} must stay hand-mineable"
            );
        }
        // Unknown blocks are not gated (fallback semantics).
        assert!(!requires_tool_for_drops("unknown_block_xyz"));
    }
}
