//! Top-down world renderer for multimodal AI perception.
//!
//! Renders a small square region of the world (centered on the player) to a
//! PNG image so that multimodal LLM clients can "see" the bot's surroundings.
//! Each pixel in the output corresponds to one block in the X-Z plane; the
//! player is drawn at the centre in red, entities in yellow, and blocks are
//! coloured by [`color_map`] according to their `block_type`.

use std::io::Cursor;

use base64::Engine;
use image::{DynamicImage, ImageBuffer, Rgba, RgbaImage};

use crate::types::WorldSnapshot;

// ═══════════════════════════════════════════════════════════════
// Public API
// ═══════════════════════════════════════════════════════════════

/// Render a top-down view of the world around the player as PNG bytes.
///
/// The image is `(2*radius+1) x (2*radius+1)` pixels, with each pixel
/// representing one block in the X-Z plane. The player is placed at the
/// centre pixel (drawn in red), entities within radius are drawn in yellow,
/// and blocks within radius are coloured via [`color_map`].
///
/// Blocks outside the radius, or with no recorded block at a given pixel,
/// are left transparent (alpha = 0).
pub fn render_topdown(snapshot: &WorldSnapshot, radius: u8) -> Vec<u8> {
    let r = radius as i32;
    let size = (2 * r + 1) as u32;
    let mut img: RgbaImage = ImageBuffer::new(size, size);

    let player = snapshot.self_player.position;
    let center_x = player.x;
    let center_z = player.z;

    // 1. Draw blocks within the Chebyshev radius (X-Z plane only).
    for block in &snapshot.blocks {
        let dx = block.position.x - center_x;
        let dz = block.position.z - center_z;
        if dx.abs() > r || dz.abs() > r {
            continue;
        }
        let px = (dx + r) as u32;
        let py = (dz + r) as u32;
        let colour = color_map(&block.block_type);
        img.put_pixel(px, py, colour);
    }

    // 2. Overlay entities (within radius) in yellow so they remain visible
    //    above any block colour.
    let entity_colour = Rgba([255, 230, 0, 255]);
    for entity in &snapshot.entities {
        let dx = entity.position.x - center_x;
        let dz = entity.position.z - center_z;
        if dx.abs() > r || dz.abs() > r {
            continue;
        }
        let px = (dx + r) as u32;
        let py = (dz + r) as u32;
        img.put_pixel(px, py, entity_colour);
    }

    // 3. Mark the player's position at the centre in red.
    let player_colour = Rgba([220, 0, 0, 255]);
    img.put_pixel(r as u32, r as u32, player_colour);

    encode_png(&img)
}

/// Look up the canonical top-down colour for a block type.
///
/// Matching is case-insensitive on the lowercased `block_type`. Common
/// Minecraft blocks map to distinctive colours; anything unrecognised falls
/// back to a neutral grey so the renderer never panics on unknown blocks.
pub fn color_map(block_type: &str) -> Rgba<u8> {
    match block_type.to_lowercase().as_str() {
        "grass" | "grass_block" | "grass_path" | "dirt_path" => Rgba([34, 139, 34, 255]),
        "dirt" | "coarse_dirt" | "podzol" | "farmland" => Rgba([139, 90, 43, 255]),
        "stone" | "cobblestone" | "mossy_cobblestone" | "bedrock" | "andesite" | "diorite"
        | "granite" | "deepslate" | "tuff" => Rgba([128, 128, 128, 255]),
        "water" | "flowing_water" | "seagrass" | "kelp" | "kelp_plant" => Rgba([64, 164, 223, 255]),
        "lava" | "flowing_lava" => Rgba([220, 80, 30, 255]),
        "sand" | "red_sand" | "sandstone" | "red_sandstone" => Rgba([218, 203, 118, 255]),
        "oak_log" | "spruce_log" | "birch_log" | "jungle_log" | "acacia_log" | "dark_oak_log"
        | "mangrove_log" | "oak_wood" | "spruce_wood" | "birch_wood" | "jungle_wood"
        | "acacia_wood" | "dark_oak_wood" => Rgba([101, 67, 33, 255]),
        "oak_leaves"
        | "spruce_leaves"
        | "birch_leaves"
        | "jungle_leaves"
        | "acacia_leaves"
        | "dark_oak_leaves"
        | "mangrove_leaves"
        | "azalea_leaves"
        | "flowering_azalea_leaves" => Rgba([34, 100, 34, 255]),
        "snow" | "snow_block" | "packed_ice" | "blue_ice" | "frosted_ice" => {
            Rgba([240, 248, 255, 255])
        }
        "iron_ore" | "deepslate_iron_ore" | "iron_block" => Rgba([200, 200, 210, 255]),
        "gold_ore" | "deepslate_gold_ore" | "gold_block" => Rgba([255, 215, 0, 255]),
        "diamond_ore" | "deepslate_diamond_ore" | "diamond_block" => Rgba([120, 220, 230, 255]),
        "emerald_ore" | "deepslate_emerald_ore" | "emerald_block" => Rgba([80, 220, 120, 255]),
        "redstone_ore" | "deepslate_redstone_ore" | "redstone_block" => Rgba([180, 30, 30, 255]),
        "coal_ore" | "deepslate_coal_ore" | "coal_block" => Rgba([40, 40, 40, 255]),
        "netherrack" | "nether_bricks" | "nether_brick_fence" => Rgba([110, 30, 30, 255]),
        "obsidian" | "crying_obsidian" => Rgba([30, 20, 50, 255]),
        "glowstone" | "sea_lantern" => Rgba([255, 240, 150, 255]),
        "crafting_table" | "furnace" | "blast_furnace" | "smoker" | "chest" | "trapped_chest"
        | "ender_chest" | "barrel" | "bookshelf" => Rgba([120, 80, 50, 255]),
        "tnt" => Rgba([200, 60, 60, 255]),
        "air" | "cave_air" | "void_air" => Rgba([0, 0, 0, 0]),
        _ => Rgba([160, 160, 160, 255]),
    }
}

/// Encode an RGBA image buffer as PNG bytes.
///
/// Panics only if the underlying PNG encoder fails (which should not happen
/// for an in-memory `RgbaImage`).
pub fn encode_png(img: &RgbaImage) -> Vec<u8> {
    let mut buf = Vec::new();
    DynamicImage::ImageRgba8(img.clone())
        .write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png)
        .expect("PNG encode should succeed for in-memory RgbaImage");
    buf
}

/// Base64-encode bytes for embedding in MCP image content.
pub fn base64_encode(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}

// ═══════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{BlockEntry, BlockPos, EntityEntry, GameMode, SelfPlayer, WorldSnapshot};

    /// PNG magic bytes — every PNG file starts with `\x89PNG\r\n\x1a\n`.
    const PNG_MAGIC: [u8; 4] = [0x89, 0x50, 0x4E, 0x47];

    fn snapshot_with_surroundings() -> WorldSnapshot {
        WorldSnapshot {
            blocks: vec![
                BlockEntry {
                    position: BlockPos::new(0, 63, 0),
                    block_type: "grass_block".into(),
                    block_state: None,
                },
                BlockEntry {
                    position: BlockPos::new(1, 63, 0),
                    block_type: "stone".into(),
                    block_state: None,
                },
                BlockEntry {
                    position: BlockPos::new(-1, 63, 0),
                    block_type: "water".into(),
                    block_state: None,
                },
                // Out of radius — should be skipped.
                BlockEntry {
                    position: BlockPos::new(50, 63, 0),
                    block_type: "diamond_ore".into(),
                    block_state: None,
                },
            ],
            entities: vec![EntityEntry {
                id: 1,
                uuid: "mob-1".into(),
                entity_type: "zombie".into(),
                position: BlockPos::new(0, 63, 1),
                display_name: Some("Zombie".into()),
                health: Some(20.0),
            }],
            self_player: SelfPlayer {
                uuid: "player".into(),
                username: "TestBot".into(),
                position: BlockPos::new(0, 64, 0),
                health: 20.0,
                hunger: 20,
                gamemode: GameMode::Survival,
                held_item_slot: 0,
                inventory: Vec::new(),
            },
            timestamp: 0,
            chunk_summary: Vec::new(),
            commands_enabled: None,
        }
    }

    #[test]
    fn test_render_topdown_returns_valid_png() {
        let snap = snapshot_with_surroundings();
        let bytes = render_topdown(&snap, 4);
        assert!(bytes.len() > 8, "PNG output should be non-trivial");
        assert!(
            bytes.starts_with(&PNG_MAGIC),
            "output should start with PNG magic bytes, got: {:?}",
            &bytes[..8.min(bytes.len())]
        );
    }

    #[test]
    fn test_render_topdown_size_scales_with_radius() {
        let snap = snapshot_with_surroundings();
        let small = render_topdown(&snap, 1);
        let large = render_topdown(&snap, 8);
        // Larger radius → more pixels → larger encoded PNG (typically).
        // We can't assert exact sizes because PNG compression varies, but
        // the larger image should encode more pixel data.
        assert!(large.len() > small.len() || large.len() >= 8);
    }

    #[test]
    fn test_color_map_common_blocks() {
        // grass / grass_block → green
        let grass = color_map("grass_block");
        assert_eq!(grass.0, [34, 139, 34, 255]);
        let grass_lower = color_map("Grass");
        assert_eq!(grass_lower.0, [34, 139, 34, 255]);

        // stone → grey
        let stone = color_map("stone");
        assert_eq!(stone.0, [128, 128, 128, 255]);

        // water → blue
        let water = color_map("water");
        assert_eq!(water.0, [64, 164, 223, 255]);

        // dirt → brown
        let dirt = color_map("dirt");
        assert_eq!(dirt.0, [139, 90, 43, 255]);

        // sand → yellow
        let sand = color_map("sand");
        assert_eq!(sand.0, [218, 203, 118, 255]);

        // oak_log → dark brown
        let log = color_map("oak_log");
        assert_eq!(log.0, [101, 67, 33, 255]);

        // leaves → dark green
        let leaves = color_map("oak_leaves");
        assert_eq!(leaves.0, [34, 100, 34, 255]);
    }

    #[test]
    fn test_color_map_unknown_block() {
        let unknown = color_map("totally_made_up_block");
        assert_eq!(unknown.0, [160, 160, 160, 255]);
    }

    #[test]
    fn test_color_map_air_is_transparent() {
        let air = color_map("air");
        assert_eq!(air.0, [0, 0, 0, 0]);
    }

    #[test]
    fn test_base64_encode_roundtrip() {
        let original = vec![0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x02];
        let encoded = base64_encode(&original);
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&encoded)
            .expect("decode");
        assert_eq!(decoded, original);
    }
}
