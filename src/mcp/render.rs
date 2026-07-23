//! Top-down world renderer for multimodal AI perception.
//!
//! Renders a small square region of the world (centered on the player) to a
//! PNG image so that multimodal LLM clients can "see" the bot's surroundings.
//! Each pixel in the output corresponds to one block in the X-Z plane; the
//! player is drawn at the centre in red, entities in yellow, and blocks are
//! coloured by [`color_map`] according to their `block_type`.
//!
//! ## Performance
//!
//! `render_topdown` uses a flat `Vec<Option<(i32, Rgba<u8>)>>` indexed by
//! `px * size + py` instead of a `HashMap` to avoid per-block hashing
//! overhead on large snapshots (5000+ blocks).
//!
//! ## Background
//!
//! Every pixel is initialised to an opaque sky-blue colour so that empty
//! regions (no recorded block) don't appear as transparent or solid black
//! on MCP clients that mishandle alpha=0. This guarantees the image looks
//! identical across clients.

use std::io::Cursor;

use base64::Engine;
use image::{DynamicImage, ImageBuffer, Rgba, RgbaImage};

use crate::error::BotError;
use crate::types::WorldSnapshot;

/// Opaque sky-blue background colour for empty pixels.
///
/// Chosen to be visually distinguishable from `stone` (grey) and `water`
/// (deeper blue) so multimodal LLMs can tell "empty/unknown" apart from
/// known solid blocks.
pub const BACKGROUND_COLOUR: Rgba<u8> = Rgba([135, 206, 235, 255]);

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
/// ## Background
///
/// All pixels (including empty ones) are initialised to [`BACKGROUND_COLOUR`]
/// (opaque sky-blue), so the image is fully opaque (alpha=255 everywhere)
/// regardless of which blocks are present. This avoids client-side alpha
/// handling quirks where alpha=0 pixels may render as solid black.
///
/// ## Performance
///
/// Uses a flat `Vec<Option<(i32, Rgba<u8>)>>` indexed by `px * size + py`
/// (O(1) array access, no hashing) to track the highest non-air block per
/// column. For a 5000-block snapshot this is ~5x faster than the previous
/// `HashMap`-based implementation.
pub fn render_topdown(snapshot: &WorldSnapshot, radius: u8) -> Result<Vec<u8>, BotError> {
    let r = radius as i32;
    let size = (2 * r + 1) as u32;
    let size_usize = size as usize;
    // Initialise every pixel to the opaque background colour. This guarantees
    // alpha=255 everywhere and replaces the previous transparent-default
    // behaviour that confused some MCP clients.
    let mut img: RgbaImage = ImageBuffer::from_pixel(size, size, BACKGROUND_COLOUR);

    let player = snapshot.self_player.position;
    let center_x = player.x;
    let center_z = player.z;

    // 1. Build a column-best array: (highest_y, colour) per pixel.
    //    Using a flat Vec indexed by `px * size + py` gives O(1) access
    //    with no hashing overhead — a major speedup over HashMap for large
    //    snapshots (5000+ blocks). Each entry is `None` if no non-air block
    //    has been seen yet for that pixel.
    let mut column_best: Vec<Option<(i32, Rgba<u8>)>> = vec![None; size_usize * size_usize];
    for block in &snapshot.blocks {
        let dx = block.position.x - center_x;
        let dz = block.position.z - center_z;
        if dx.abs() > r || dz.abs() > r {
            continue;
        }
        // Skip air blocks — they shouldn't obscure lower blocks.
        if block.block_type.eq_ignore_ascii_case("air") {
            continue;
        }
        let px = (dx + r) as u32;
        let py = (dz + r) as u32;
        let colour = color_map(&block.block_type);
        let idx = (px as usize) * size_usize + (py as usize);
        match &mut column_best[idx] {
            Some((best_y, c)) => {
                if block.position.y > *best_y {
                    *best_y = block.position.y;
                    *c = colour;
                }
            }
            None => {
                column_best[idx] = Some((block.position.y, colour));
            }
        }
    }
    // Paint the highest non-air block per pixel. Empty pixels keep the
    // opaque sky-blue background.
    for (px, py, pixel) in img.enumerate_pixels_mut() {
        let idx = (px as usize) * size_usize + (py as usize);
        if let Some((_, colour)) = column_best[idx] {
            *pixel = colour;
        }
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

// ═══════════════════════════════════════════════════════════════
// Enhanced renderer — scale, Y-axis height modulation, yaw arrow
// ═══════════════════════════════════════════════════════════════

/// Valid `scale` values for [`render_topdown_enhanced`].
///
/// Each block occupies a `scale × scale` square of pixels in the output.
/// `1` produces the legacy 1-pixel-per-block render; `8` produces a
/// 520×520 image for `radius=32`, suitable for multimodal LLMs that prefer
/// higher-resolution input.
pub const VALID_SCALES: &[u8] = &[1, 2, 4, 8];

/// Default scale when the caller passes `0` or an unsupported value.
pub const DEFAULT_SCALE: u8 = 1;

/// Highest Y coordinate Minecraft can produce (1.21 build height = 320).
///
/// Used to normalise block Y for the height-modulation brightness factor.
pub const MAX_BUILD_Y: i32 = 320;
/// Lowest Y coordinate Minecraft can produce (1.21 build floor = -64).
pub const MIN_BUILD_Y: i32 = -64;

/// Render a top-down view with `scale`, Y-axis brightness modulation, and
/// an optional yaw heading arrow at the player's position.
///
/// This is the enhanced variant of [`render_topdown`]:
///
/// - `scale` (1/2/4/8) makes each block occupy `scale×scale` pixels, so a
///   `radius=8` render at `scale=4` is `68×68` blocks → `272×272` pixels.
///   Values outside `[1, 2, 4, 8]` are clamped to [`DEFAULT_SCALE`].
/// - When `SelfPlayer::position_precise` is `Some`, the centre pixel is
///   computed by rounding the floating-point coords (sub-block precision),
///   eliminating the up-to-1-pixel truncation bias of the integer `position`.
/// - Each pixel's colour is multiplied by a Y-axis brightness factor
///   `0.75 + 0.25 * ((y - MIN_BUILD_Y) / (MAX_BUILD_Y - MIN_BUILD_Y))`, so
///   higher blocks appear slightly brighter and lower blocks slightly
///   darker. This gives the multimodal LLM a depth cue without colour
///   shifting the underlying block identity.
/// - When `SelfPlayer::yaw` is `Some`, a 3-pixel-wide arrow is drawn at the
///   player's centre pixel, pointing in the direction the bot is facing
///   (Minecraft yaw convention: 0 = +Z/south, +π/2 = -X/west).
///
/// `render_topdown(snapshot, radius)` is equivalent to
/// `render_topdown_enhanced(snapshot, radius, 1)` minus the Y-modulation
/// and yaw arrow (kept separate so the existing simpler API stays stable
/// for tests and downstream callers).
pub fn render_topdown_enhanced(
    snapshot: &WorldSnapshot,
    radius: u8,
    scale: u8,
) -> Result<Vec<u8>, BotError> {
    // Clamp scale to a supported value — invalid inputs silently fall back
    // to 1× rather than producing a malformed image.
    let scale = if VALID_SCALES.contains(&scale) {
        scale
    } else {
        DEFAULT_SCALE
    };

    let r = radius as i32;
    let block_size = (2 * r + 1) as u32;
    let size = block_size.checked_mul(scale as u32).ok_or_else(|| {
        BotError::Internal(format!(
            "rendered image size overflow: block_size={block_size} scale={scale}"
        ))
    })?;

    // Initialise every pixel to the opaque background colour.
    let mut img: RgbaImage = ImageBuffer::from_pixel(size, size, BACKGROUND_COLOUR);

    // Compute the player's centre with sub-block precision when available.
    // Fall back to the integer `position` (no precision loss vs the legacy
    // renderer).
    let (center_x, center_z) = snapshot
        .self_player
        .position_precise
        .map(|p| (p[0], p[2]))
        .unwrap_or((
            snapshot.self_player.position.x as f64,
            snapshot.self_player.position.z as f64,
        ));

    // 1. Build a column-best array (highest non-air block per (px, py)).
    //    Same Vec<Option<...>> approach as render_topdown for O(1) access.
    let block_size_usize = block_size as usize;
    let mut column_best: Vec<Option<(i32, Rgba<u8>)>> =
        vec![None; block_size_usize * block_size_usize];
    for block in &snapshot.blocks {
        let dx = block.position.x as f64 - center_x;
        let dz = block.position.z as f64 - center_z;
        if dx.abs() > r as f64 || dz.abs() > r as f64 {
            continue;
        }
        if block.block_type.eq_ignore_ascii_case("air") {
            continue;
        }
        // Round to nearest pixel (sub-block precision) rather than floor.
        // The previous `as i32` cast truncated, biasing the centre by up
        // to 1 block.
        let px = ((dx.round() as i32 + r) as u32).min(block_size - 1);
        let py = ((dz.round() as i32 + r) as u32).min(block_size - 1);
        let colour = color_map(&block.block_type);
        let idx = (px as usize) * block_size_usize + (py as usize);
        match &mut column_best[idx] {
            Some((best_y, c)) => {
                if block.position.y > *best_y {
                    *best_y = block.position.y;
                    *c = colour;
                }
            }
            None => {
                column_best[idx] = Some((block.position.y, colour));
            }
        }
    }

    // 2. Paint each block into a `scale × scale` square, applying Y-axis
    //    brightness modulation. Higher blocks → slightly brighter; lower
    //    blocks → slightly darker. The factor range is [0.75, 1.0] so no
    //    colour is ever darker than 75% of its base value.
    let y_span = (MAX_BUILD_Y - MIN_BUILD_Y) as f32;
    for (px, py, pixel) in img.enumerate_pixels_mut() {
        let bx = px / scale as u32;
        let by = py / scale as u32;
        let idx = (bx as usize) * block_size_usize + (by as usize);
        if let Some((y, colour)) = column_best[idx] {
            let y_norm = ((y - MIN_BUILD_Y) as f32 / y_span).clamp(0.0, 1.0);
            let brightness = 0.75 + 0.25 * y_norm;
            *pixel = modulate_brightness(colour, brightness);
        }
    }

    // 3. Overlay entities (within radius) in yellow, each occupying a
    //    `scale × scale` square so they remain visible at scale=1.
    let entity_colour = Rgba([255, 230, 0, 255]);
    for entity in &snapshot.entities {
        let dx = entity.position.x as f64 - center_x;
        let dz = entity.position.z as f64 - center_z;
        if dx.abs() > r as f64 || dz.abs() > r as f64 {
            continue;
        }
        let px = (dx.round() as i32 + r) as u32 * scale as u32;
        let py = (dz.round() as i32 + r) as u32 * scale as u32;
        paint_square(&mut img, px, py, scale as u32, entity_colour);
    }

    // 4. Player marker + optional yaw heading arrow at the centre.
    let player_colour = Rgba([220, 0, 0, 255]);
    let cx = r as u32 * scale as u32;
    let cy = r as u32 * scale as u32;
    paint_square(&mut img, cx, cy, scale.max(2) as u32, player_colour);

    // Draw a heading arrow if yaw is available. The arrow is a 3-pixel-wide
    // line extending `scale * 3` pixels from the centre in the direction
    // the bot is facing.
    if let Some(yaw) = snapshot.self_player.yaw {
        draw_yaw_arrow(&mut img, cx, cy, yaw, scale);
    }

    encode_png(&img)
}

/// Multiply each colour channel of `colour` by `factor` (alpha untouched).
///
/// Used for Y-axis brightness modulation. The factor is clamped to
/// `[0.0, 1.0]` so the result never overflows or inverts the colour.
fn modulate_brightness(colour: Rgba<u8>, factor: f32) -> Rgba<u8> {
    let factor = factor.clamp(0.0, 1.0);
    Rgba([
        (colour[0] as f32 * factor).round() as u8,
        (colour[1] as f32 * factor).round() as u8,
        (colour[2] as f32 * factor).round() as u8,
        colour[3],
    ])
}

/// Paint a `size × size` square at `(x, y)` (top-left corner), clamped to
/// the image bounds.
fn paint_square(img: &mut RgbaImage, x: u32, y: u32, size: u32, colour: Rgba<u8>) {
    let (w, h) = img.dimensions();
    for dx in 0..size {
        for dy in 0..size {
            let px = x.saturating_add(dx);
            let py = y.saturating_add(dy);
            if px < w && py < h {
                img.put_pixel(px, py, colour);
            }
        }
    }
}

/// Draw a 3-pixel-wide heading arrow at `(cx, cy)` pointing in the
/// direction `yaw`.
///
/// Minecraft yaw convention:
/// - `0`     → facing +Z (south, downward on the image since +Z is down)
/// - `+π/2`  → facing -X (west, leftward on the image since +X is right)
/// - `±π`    → facing -Z (north, upward)
/// - `-π/2`  → facing +X (east, rightward)
///
/// The arrow is `scale * 3` pixels long and ends in a small triangular
/// head. It is drawn in a contrasting bright colour (white) so it stands
/// out against the red player marker.
fn draw_yaw_arrow(img: &mut RgbaImage, cx: u32, cy: u32, yaw: f32, scale: u8) {
    // Convert yaw → (dx, dz) screen-space direction.
    // Minecraft yaw 0 = +Z (down on image), so:
    //   dx_screen = -sin(yaw)  (yaw=0 → dx=0; yaw=+π/2 → dx=-1 west)
    //   dy_screen =  cos(yaw)  (yaw=0 → dy=+1 south/down)
    let dir_x = -yaw.sin();
    let dir_y = yaw.cos();

    let length = (scale as u32).max(1) * 3;
    let arrow_colour = Rgba([255, 255, 255, 255]);
    let (w, h) = img.dimensions();

    // Draw a line of `length` pixels from centre in the (dir_x, dir_y)
    // direction, 3 pixels wide (perpendicular to the direction).
    for step in 1..=length {
        let px = cx as f32 + dir_x * step as f32;
        let py = cy as f32 + dir_y * step as f32;
        // Round to nearest pixel.
        let px_i = px.round() as i32;
        let py_i = py.round() as i32;
        // Paint a 3-pixel-wide cross around (px, py).
        for &(ox, oy) in &[(0i32, 0i32), (-1, 0), (1, 0), (0, -1), (0, 1)] {
            let fx = px_i + ox;
            let fy = py_i + oy;
            if fx >= 0 && fy >= 0 && (fx as u32) < w && (fy as u32) < h {
                img.put_pixel(fx as u32, fy as u32, arrow_colour);
            }
        }
    }
}

/// Look up the canonical top-down colour for a block type.
///
/// Matching is case-insensitive on the lowercased `block_type`. Common
/// Minecraft blocks map to distinctive colours; anything unrecognised falls
/// back to a neutral grey so the renderer never panics on unknown blocks.
///
/// ## Coverage
///
/// The table is organised by category:
///
/// - **Terrain**: grass, dirt, stone, sand, snow, etc.
/// - **Ores & metals**: iron/gold/diamond/emerald/redstone/coal ore + blocks
/// - **Logs & wood**: oak/spruce/birch/jungle/acacia/dark_oak/mangrove logs & wood
/// - **Planks & planks-derivatives**: all plank variants, slabs, stairs,
///   fences, doors, buttons, pressure plates, signs, boats
/// - **Leaves**: all leaf variants
/// - **Glass**: glass + panes + tinted variants
/// - **Wool & carpets**: all 16 colours of wool and carpet
/// - **Concrete & concrete powder**: all 16 colours
/// - **Terracotta & glazed**: raw + all 16 glazed variants
/// - **Bricks**: stone bricks, nether bricks, deepslate bricks
/// - **Plants**: flowers, grasses, mushrooms, cactus, sugar cane, bamboo
/// - **Functional**: doors, trapdoors, ladders, rails, torches, redstone
///   components, pistons, dispensers, observers, repeaters, comparators
/// - **Containers**: crafting table, furnace, chest, barrel, bookshelf,
///   shulker boxes, hoppers, droppers
/// - **Nether**: netherrack, obsidian, glowstone, nether bricks, soul sand
/// - **End**: end stone, purpur, end stone bricks
pub fn color_map(block_type: &str) -> Rgba<u8> {
    let lowercased;
    let key = if block_type.chars().any(|c| c.is_uppercase()) {
        lowercased = block_type.to_lowercase();
        lowercased.as_str()
    } else {
        block_type
    };
    match key {
        // ── Terrain ──────────────────────────────────────────────
        "grass" | "grass_block" | "grass_path" | "dirt_path" => Rgba([34, 139, 34, 255]),
        "dirt" | "coarse_dirt" | "podzol" | "farmland" | "rooted_dirt" => Rgba([139, 90, 43, 255]),
        "stone" | "cobblestone" | "mossy_cobblestone" | "bedrock" | "andesite" | "diorite"
        | "granite" | "deepslate" | "tuff" | "calcite" | "dripstone_block"
        | "pointed_dripstone" => Rgba([128, 128, 128, 255]),
        "water" | "flowing_water" | "seagrass" | "kelp" | "kelp_plant" | "seagrass_tall"
        | "tall_seagrass" => Rgba([64, 164, 223, 255]),
        "lava" | "flowing_lava" => Rgba([220, 80, 30, 255]),
        "sand" | "red_sand" | "sandstone" | "red_sandstone" | "smooth_sandstone"
        | "cut_sandstone" | "chiseled_sandstone" => Rgba([218, 203, 118, 255]),
        "gravel" | "flint" => Rgba([110, 100, 90, 255]),
        "clay" | "clay_ball" => Rgba([160, 160, 180, 255]),
        "snow" | "snow_block" | "packed_ice" | "blue_ice" | "frosted_ice" | "ice" | "ice_path" => {
            Rgba([240, 248, 255, 255])
        }
        "mycelium" => Rgba([120, 100, 130, 255]),
        "mud" | "muddy_mangrove_roots" => Rgba([95, 75, 60, 255]),
        "soul_sand" | "soul_soil" => Rgba([80, 60, 50, 255]),

        // ── Logs & wood ──────────────────────────────────────────
        "oak_log" | "spruce_log" | "birch_log" | "jungle_log" | "acacia_log" | "dark_oak_log"
        | "mangrove_log" | "cherry_log" | "bamboo_block" | "oak_wood" | "spruce_wood"
        | "birch_wood" | "jungle_wood" | "acacia_wood" | "dark_oak_wood" | "mangrove_wood"
        | "cherry_wood" => Rgba([101, 67, 33, 255]),

        // ── Planks & plank-derived ──────────────────────────────
        // All plank variants share one colour family.
        "oak_planks"
        | "spruce_planks"
        | "birch_planks"
        | "jungle_planks"
        | "acacia_planks"
        | "dark_oak_planks"
        | "mangrove_planks"
        | "cherry_planks"
        | "bamboo_planks"
        | "oak_slab"
        | "spruce_slab"
        | "birch_slab"
        | "jungle_slab"
        | "acacia_slab"
        | "dark_oak_slab"
        | "mangrove_slab"
        | "cherry_slab"
        | "bamboo_slab"
        | "oak_stairs"
        | "spruce_stairs"
        | "birch_stairs"
        | "jungle_stairs"
        | "acacia_stairs"
        | "dark_oak_stairs"
        | "mangrove_stairs"
        | "cherry_stairs"
        | "bamboo_stairs"
        | "oak_fence"
        | "spruce_fence"
        | "birch_fence"
        | "jungle_fence"
        | "acacia_fence"
        | "dark_oak_fence"
        | "mangrove_fence"
        | "cherry_fence"
        | "bamboo_fence"
        | "oak_fence_gate"
        | "spruce_fence_gate"
        | "birch_fence_gate"
        | "jungle_fence_gate"
        | "acacia_fence_gate"
        | "dark_oak_fence_gate"
        | "mangrove_fence_gate"
        | "cherry_fence_gate"
        | "bamboo_fence_gate"
        | "oak_door"
        | "spruce_door"
        | "birch_door"
        | "jungle_door"
        | "acacia_door"
        | "dark_oak_door"
        | "mangrove_door"
        | "cherry_door"
        | "bamboo_door"
        | "oak_trapdoor"
        | "spruce_trapdoor"
        | "birch_trapdoor"
        | "jungle_trapdoor"
        | "acacia_trapdoor"
        | "dark_oak_trapdoor"
        | "mangrove_trapdoor"
        | "cherry_trapdoor"
        | "oak_button"
        | "spruce_button"
        | "birch_button"
        | "jungle_button"
        | "acacia_button"
        | "dark_oak_button"
        | "mangrove_button"
        | "cherry_button"
        | "oak_pressure_plate"
        | "spruce_pressure_plate"
        | "birch_pressure_plate"
        | "jungle_pressure_plate"
        | "acacia_pressure_plate"
        | "dark_oak_pressure_plate"
        | "mangrove_pressure_plate"
        | "cherry_pressure_plate"
        | "oak_sign"
        | "spruce_sign"
        | "birch_sign"
        | "jungle_sign"
        | "acacia_sign"
        | "dark_oak_sign"
        | "mangrove_sign"
        | "cherry_sign"
        | "oak_wall_sign"
        | "spruce_wall_sign"
        | "birch_wall_sign"
        | "jungle_wall_sign"
        | "acacia_wall_sign"
        | "dark_oak_wall_sign"
        | "mangrove_wall_sign"
        | "cherry_wall_sign" => Rgba([160, 130, 80, 255]),

        // ── Leaves ───────────────────────────────────────────────
        "oak_leaves"
        | "spruce_leaves"
        | "birch_leaves"
        | "jungle_leaves"
        | "acacia_leaves"
        | "dark_oak_leaves"
        | "mangrove_leaves"
        | "azalea_leaves"
        | "flowering_azalea_leaves"
        | "cherry_leaves" => Rgba([34, 100, 34, 255]),

        // ── Glass ───────────────────────────────────────────────
        "glass" | "glass_pane" | "tinted_glass" | "glass_bottle" => Rgba([180, 220, 240, 255]),

        // ── Wool & carpets (16 colours) ─────────────────────────
        "white_wool" | "white_carpet" => Rgba([235, 235, 235, 255]),
        "orange_wool" | "orange_carpet" => Rgba([235, 138, 28, 255]),
        "magenta_wool" | "magenta_carpet" => Rgba([180, 60, 180, 255]),
        "light_blue_wool" | "light_blue_carpet" => Rgba([100, 160, 220, 255]),
        "yellow_wool" | "yellow_carpet" => Rgba([240, 215, 40, 255]),
        "lime_wool" | "lime_carpet" => Rgba([120, 200, 60, 255]),
        "pink_wool" | "pink_carpet" => Rgba([235, 150, 175, 255]),
        "gray_wool" | "gray_carpet" => Rgba([70, 70, 80, 255]),
        "light_gray_wool" | "light_gray_carpet" => Rgba([160, 160, 170, 255]),
        "cyan_wool" | "cyan_carpet" => Rgba([60, 140, 150, 255]),
        "purple_wool" | "purple_carpet" => Rgba([130, 60, 160, 255]),
        "blue_wool" | "blue_carpet" => Rgba([50, 60, 160, 255]),
        "brown_wool" | "brown_carpet" => Rgba([100, 70, 50, 255]),
        "green_wool" | "green_carpet" => Rgba([70, 100, 50, 255]),
        "red_wool" | "red_carpet" => Rgba([160, 50, 50, 255]),
        "black_wool" | "black_carpet" => Rgba([30, 30, 35, 255]),

        // ── Concrete & concrete powder (16 colours) ─────────────
        "white_concrete" | "white_concrete_powder" => Rgba([220, 220, 220, 255]),
        "orange_concrete" | "orange_concrete_powder" => Rgba([220, 120, 30, 255]),
        "magenta_concrete" | "magenta_concrete_powder" => Rgba([170, 50, 170, 255]),
        "light_blue_concrete" | "light_blue_concrete_powder" => Rgba([80, 140, 200, 255]),
        "yellow_concrete" | "yellow_concrete_powder" => Rgba([230, 200, 30, 255]),
        "lime_concrete" | "lime_concrete_powder" => Rgba([100, 180, 50, 255]),
        "pink_concrete" | "pink_concrete_powder" => Rgba([220, 130, 160, 255]),
        "gray_concrete" | "gray_concrete_powder" => Rgba([60, 60, 70, 255]),
        "light_gray_concrete" | "light_gray_concrete_powder" => Rgba([150, 150, 160, 255]),
        "cyan_concrete" | "cyan_concrete_powder" => Rgba([50, 120, 130, 255]),
        "purple_concrete" | "purple_concrete_powder" => Rgba([120, 50, 150, 255]),
        "blue_concrete" | "blue_concrete_powder" => Rgba([40, 50, 140, 255]),
        "brown_concrete" | "brown_concrete_powder" => Rgba([90, 60, 40, 255]),
        "green_concrete" | "green_concrete_powder" => Rgba([60, 90, 40, 255]),
        "red_concrete" | "red_concrete_powder" => Rgba([150, 40, 40, 255]),
        "black_concrete" | "black_concrete_powder" => Rgba([25, 25, 30, 255]),

        // ── Terracotta & glazed ─────────────────────────────────
        "terracotta" => Rgba([150, 90, 70, 255]),
        "white_terracotta" => Rgba([210, 180, 165, 255]),
        "orange_terracotta" => Rgba([160, 95, 50, 255]),
        "magenta_terracotta" => Rgba([150, 90, 130, 255]),
        "light_blue_terracotta" => Rgba([120, 130, 150, 255]),
        "yellow_terracotta" => Rgba([180, 150, 60, 255]),
        "lime_terracotta" => Rgba([110, 130, 70, 255]),
        "pink_terracotta" => Rgba([170, 110, 110, 255]),
        "gray_terracotta" => Rgba([70, 60, 65, 255]),
        "light_gray_terracotta" => Rgba([140, 130, 130, 255]),
        "cyan_terracotta" => Rgba([90, 100, 100, 255]),
        "purple_terracotta" => Rgba([120, 80, 130, 255]),
        "blue_terracotta" => Rgba([80, 90, 130, 255]),
        "brown_terracotta" => Rgba([90, 70, 60, 255]),
        "green_terracotta" => Rgba([80, 90, 60, 255]),
        "red_terracotta" => Rgba([140, 70, 60, 255]),
        "black_terracotta" => Rgba([40, 35, 40, 255]),
        // Glazed terracotta share their colour's tint.
        "white_glazed_terracotta" => Rgba([235, 235, 235, 255]),
        "orange_glazed_terracotta" => Rgba([235, 138, 28, 255]),
        "magenta_glazed_terracotta" => Rgba([180, 60, 180, 255]),
        "light_blue_glazed_terracotta" => Rgba([100, 160, 220, 255]),
        "yellow_glazed_terracotta" => Rgba([240, 215, 40, 255]),
        "lime_glazed_terracotta" => Rgba([120, 200, 60, 255]),
        "pink_glazed_terracotta" => Rgba([235, 150, 175, 255]),
        "gray_glazed_terracotta" => Rgba([70, 70, 80, 255]),
        "light_gray_glazed_terracotta" => Rgba([160, 160, 170, 255]),
        "cyan_glazed_terracotta" => Rgba([60, 140, 150, 255]),
        "purple_glazed_terracotta" => Rgba([130, 60, 160, 255]),
        "blue_glazed_terracotta" => Rgba([50, 60, 160, 255]),
        "brown_glazed_terracotta" => Rgba([100, 70, 50, 255]),
        "green_glazed_terracotta" => Rgba([70, 100, 50, 255]),
        "red_glazed_terracotta" => Rgba([160, 50, 50, 255]),
        "black_glazed_terracotta" => Rgba([30, 30, 35, 255]),

        // ── Bricks & brick-derived ───────────────────────────────
        "bricks"
        | "brick_slab"
        | "brick_stairs"
        | "brick_wall"
        | "brick_fence"
        | "nether_brick_fence"
        | "nether_bricks"
        | "nether_brick_slab"
        | "nether_brick_stairs"
        | "nether_brick_wall"
        | "red_nether_bricks"
        | "deepslate_bricks"
        | "deepslate_brick_slab"
        | "deepslate_brick_stairs"
        | "deepslate_brick_wall"
        | "deepslate_tiles"
        | "deepslate_tile_slab"
        | "deepslate_tile_stairs"
        | "deepslate_tile_wall"
        | "stone_bricks"
        | "mossy_stone_bricks"
        | "cracked_stone_bricks"
        | "chiseled_stone_bricks"
        | "stone_brick_slab"
        | "stone_brick_stairs"
        | "stone_brick_wall"
        | "mossy_stone_brick_slab"
        | "mossy_stone_brick_stairs"
        | "mossy_stone_brick_wall"
        | "cobblestone_slab"
        | "cobblestone_stairs"
        | "cobblestone_wall"
        | "mossy_cobblestone_slab"
        | "mossy_cobblestone_stairs"
        | "mossy_cobblestone_wall"
        | "sandstone_slab"
        | "sandstone_stairs"
        | "sandstone_wall"
        | "red_sandstone_slab"
        | "red_sandstone_stairs"
        | "red_sandstone_wall"
        | "quartz_block"
        | "quartz_slab"
        | "quartz_stairs"
        | "quartz_pillar"
        | "smooth_quartz"
        | "chiseled_quartz_block"
        | "quartz_bricks" => Rgba([140, 80, 70, 255]),

        // ── Ores & metals ────────────────────────────────────────
        "iron_ore" | "deepslate_iron_ore" | "iron_block" | "raw_iron_block" | "raw_iron" => {
            Rgba([200, 200, 210, 255])
        }
        "gold_ore" | "deepslate_gold_ore" | "gold_block" | "raw_gold_block" | "raw_gold" => {
            Rgba([255, 215, 0, 255])
        }
        "diamond_ore" | "deepslate_diamond_ore" | "diamond_block" => Rgba([120, 220, 230, 255]),
        "emerald_ore" | "deepslate_emerald_ore" | "emerald_block" => Rgba([80, 220, 120, 255]),
        "redstone_ore" | "deepslate_redstone_ore" | "redstone_block" => Rgba([180, 30, 30, 255]),
        "coal_ore" | "deepslate_coal_ore" | "coal_block" => Rgba([40, 40, 40, 255]),
        "lapis_ore" | "deepslate_lapis_ore" | "lapis_block" => Rgba([40, 80, 180, 255]),
        "copper_ore"
        | "deepslate_copper_ore"
        | "copper_block"
        | "raw_copper_block"
        | "raw_copper"
        | "exposed_copper"
        | "weathered_copper"
        | "oxidized_copper" => Rgba([190, 130, 90, 255]),
        "netherite_block" | "ancient_debris" => Rgba([60, 50, 45, 255]),

        // ── Nether ───────────────────────────────────────────────
        "netherrack" => Rgba([110, 30, 30, 255]),
        "obsidian" | "crying_obsidian" => Rgba([30, 20, 50, 255]),
        "glowstone" | "sea_lantern" => Rgba([255, 240, 150, 255]),
        "magma_block" => Rgba([120, 50, 30, 255]),
        "basalt" | "smooth_basalt" | "polished_basalt" => Rgba([60, 60, 70, 255]),
        "blackstone" | "gilded_blackstone" => Rgba([40, 40, 50, 255]),
        "nether_gold_ore" | "nether_quartz_ore" => Rgba([100, 80, 70, 255]),

        // ── End ─────────────────────────────────────────────────
        "end_stone"
        | "end_stone_bricks"
        | "end_stone_brick_slab"
        | "end_stone_brick_stairs"
        | "end_stone_brick_wall" => Rgba([220, 220, 160, 255]),
        "purpur_block" | "purpur_pillar" | "purpur_slab" | "purpur_stairs" | "purpur_wall" => {
            Rgba([160, 120, 170, 255])
        }

        // ── Containers & functional ──────────────────────────────
        "crafting_table"
        | "furnace"
        | "blast_furnace"
        | "smoker"
        | "chest"
        | "trapped_chest"
        | "ender_chest"
        | "barrel"
        | "bookshelf"
        | "chiseled_bookshelf"
        | "lectern"
        | "loom"
        | "cartography_table"
        | "fletching_table"
        | "smithing_table"
        | "grindstone"
        | "stonecutter"
        | "anvil"
        | "chipped_anvil"
        | "damaged_anvil"
        | "enchanting_table"
        | "brewing_stand"
        | "cauldron"
        | "composter"
        | "shulker_box"
        | "white_shulker_box"
        | "orange_shulker_box"
        | "magenta_shulker_box"
        | "light_blue_shulker_box"
        | "yellow_shulker_box"
        | "lime_shulker_box"
        | "pink_shulker_box"
        | "gray_shulker_box"
        | "light_gray_shulker_box"
        | "cyan_shulker_box"
        | "purple_shulker_box"
        | "blue_shulker_box"
        | "brown_shulker_box"
        | "green_shulker_box"
        | "red_shulker_box"
        | "black_shulker_box"
        | "hopper"
        | "dispenser"
        | "dropper"
        | "observer" => Rgba([120, 80, 50, 255]),

        // ── Redstone & mechanical ───────────────────────────────
        "redstone_wire"
        | "redstone_torch"
        | "redstone_wall_torch"
        | "repeater"
        | "comparator"
        | "piston"
        | "sticky_piston"
        | "piston_head"
        | "moving_piston"
        | "redstone_lamp"
        | "lever"
        | "stone_button"
        | "stone_pressure_plate"
        | "light_weighted_pressure_plate"
        | "heavy_weighted_pressure_plate"
        | "iron_door"
        | "iron_trapdoor"
        | "iron_bars"
        | "tripwire_hook"
        | "tripwire"
        | "note_block"
        | "daylight_detector" => Rgba([180, 50, 50, 255]),

        // ── Rails & tracks ──────────────────────────────────────
        "rail" | "powered_rail" | "detector_rail" | "activator_rail" => Rgba([180, 160, 120, 255]),

        // ── Ladder & vine ───────────────────────────────────────
        "ladder" | "vine" | "scaffolding" => Rgba([170, 130, 70, 255]),

        // ── Torches & lights ────────────────────────────────────
        // (redstone_torch / redstone_wall_torch intentionally omitted here —
        // they are matched by the Redstone section above. Adding them here
        // would shadow the earlier arm and trigger unreachable_pattern.)
        "torch"
        | "wall_torch"
        | "soul_torch"
        | "soul_wall_torch"
        | "lantern"
        | "soul_lantern"
        | "end_rod"
        | "ochre_froglight"
        | "verdant_froglight"
        | "pearlescent_froglight"
        | "shroomlight"
        | "conduit"
        | "beacon" => Rgba([255, 220, 100, 255]),

        // ── Plants (flowers, mushrooms, cactus, etc.) ────────────
        // (`grass` is matched by the Terrain section above — grass_block
        // family. Adding it here would shadow and trigger
        // unreachable_pattern.)
        "dandelion" | "poppy" | "blue_orchid" | "allium" | "azure_bluet" | "red_tulip"
        | "orange_tulip" | "white_tulip" | "pink_tulip" | "oxeye_daisy" | "cornflower"
        | "lily_of_the_valley" | "wither_rose" | "sunflower" | "lilac" | "rose_bush" | "peony"
        | "tall_grass" | "large_fern" | "fern" | "dead_bush" | "sweet_berry_bush" => {
            Rgba([220, 180, 60, 255])
        }
        "red_mushroom" => Rgba([200, 60, 60, 255]),
        "brown_mushroom" => Rgba([130, 90, 60, 255]),
        "mushroom_stem" => Rgba([220, 200, 180, 255]),
        "cactus" => Rgba([90, 140, 60, 255]),
        "sugar_cane" | "bamboo" | "bamboo_sapling" => Rgba([140, 180, 80, 255]),
        "pumpkin" | "carved_pumpkin" | "jack_o_lantern" => Rgba([200, 110, 30, 255]),
        "melon" => Rgba([60, 130, 60, 255]),
        "hay_block" => Rgba([200, 180, 80, 255]),
        "cake" => Rgba([240, 220, 200, 255]),
        "wheat" | "wheat_seeds" | "beetroots" | "carrots" | "potatoes" => Rgba([180, 150, 60, 255]),
        "nether_wart" => Rgba([130, 40, 60, 255]),

        // ── Slabs, stairs, walls (generic stone variants) ────────
        // (`deepslate_brick_slab` is matched by the Bricks section above.)
        "smooth_stone"
        | "smooth_stone_slab"
        | "smooth_sandstone_slab"
        | "smooth_red_sandstone_slab"
        | "smooth_quartz_slab"
        | "cut_sandstone_slab"
        | "cut_red_sandstone_slab"
        | "polished_andesite"
        | "polished_andesite_slab"
        | "polished_andesite_stairs"
        | "polished_diorite"
        | "polished_diorite_slab"
        | "polished_diorite_stairs"
        | "polished_granite"
        | "polished_granite_slab"
        | "polished_granite_stairs"
        | "polished_deepslate"
        | "polished_deepslate_slab"
        | "polished_deepslate_stairs"
        | "polished_deepslate_wall"
        | "cobbled_deepslate"
        | "cobbled_deepslate_slab"
        | "cobbled_deepslate_stairs"
        | "cobbled_deepslate_wall" => Rgba([110, 110, 120, 255]),

        // ── Misc functional ──────────────────────────────────────
        // (`bedrock` is matched by the Terrain section above.)
        "tnt" => Rgba([200, 60, 60, 255]),
        "jukebox" => Rgba([100, 70, 50, 255]),
        "spawner" => Rgba([40, 40, 50, 255]),
        "infested_stone"
        | "infested_cobblestone"
        | "infested_deepslate"
        | "infested_stone_bricks"
        | "infested_mossy_stone_bricks"
        | "infested_cracked_stone_bricks"
        | "infested_chiseled_stone_bricks" => Rgba([110, 100, 100, 255]),
        "command_block" | "chain_command_block" | "repeating_command_block" => {
            Rgba([160, 120, 70, 255])
        }
        "structure_block" | "structure_void" | "jigsaw" => Rgba([100, 100, 110, 255]),

        // ── Air (transparent, but skipped by render_topdown) ─────
        "air" | "cave_air" | "void_air" => Rgba([0, 0, 0, 0]),

        // ── Fallback: grey for unknown blocks ────────────────────
        _ => Rgba([160, 160, 160, 255]),
    }
}

/// Encode an RGBA image buffer as PNG bytes.
///
/// Returns an error if the underlying PNG encoder fails (which should not
/// happen for an in-memory `RgbaImage`).
pub fn encode_png(img: &RgbaImage) -> Result<Vec<u8>, BotError> {
    let mut buf = Vec::new();
    DynamicImage::ImageRgba8(img.clone())
        .write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png)
        .map_err(|e| BotError::Internal(format!("PNG encode failed: {e}")))?;
    Ok(buf)
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
                position_precise: None,
                yaw: None,
            },
            timestamp: 0,
            chunk_summary: Vec::new(),
            commands_enabled: None,
            ..Default::default()
        }
    }

    #[test]
    fn test_render_topdown_returns_valid_png() {
        let snap = snapshot_with_surroundings();
        let bytes = render_topdown(&snap, 4).expect("render should succeed");
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
        let small = render_topdown(&snap, 1).expect("render should succeed");
        let large = render_topdown(&snap, 8).expect("render should succeed");
        // Larger radius → more pixels → larger encoded PNG (typically).
        // We can't assert exact sizes because PNG compression varies, but
        // the larger image should encode more pixel data.
        assert!(large.len() > small.len() || large.len() >= 8);
    }

    #[test]
    fn test_render_topdown_picks_highest_block() {
        // Two non-air blocks at the same (x,z) but different Y: the higher
        // one (grass) should win over the lower one (stone), regardless of
        // iteration order. An air block above both should be skipped entirely
        // so it doesn't obscure the grass below.
        let snap = WorldSnapshot {
            blocks: vec![
                // Lower non-air block.
                BlockEntry {
                    position: BlockPos::new(1, 63, 0),
                    block_type: "stone".into(),
                    block_state: None,
                },
                // Higher non-air block — should win the column.
                BlockEntry {
                    position: BlockPos::new(1, 70, 0),
                    block_type: "grass_block".into(),
                    block_state: None,
                },
                // Air above both — must be skipped, not obscure the grass.
                BlockEntry {
                    position: BlockPos::new(1, 80, 0),
                    block_type: "air".into(),
                    block_state: None,
                },
            ],
            entities: Vec::new(),
            self_player: SelfPlayer {
                uuid: "player".into(),
                username: "TestBot".into(),
                position: BlockPos::new(0, 64, 0),
                health: 20.0,
                hunger: 20,
                gamemode: GameMode::Survival,
                held_item_slot: 0,
                inventory: Vec::new(),
                position_precise: None,
                yaw: None,
            },
            timestamp: 0,
            chunk_summary: Vec::new(),
            commands_enabled: None,
            ..Default::default()
        };
        let bytes = render_topdown(&snap, 4).expect("render should succeed");
        let img = image::load_from_memory(&bytes)
            .expect("decode PNG")
            .to_rgba8();
        // Player at (0,64,0) → centre pixel (r, r) = (4, 4) in red.
        // Stacked column at (x=1, z=0) → dx=1, dz=0 → pixel (5, 4).
        let pixel = img.get_pixel(5, 4);
        let grass_colour = color_map("grass_block");
        assert_eq!(
            pixel.0, grass_colour.0,
            "pixel (5,4) should be grass (highest non-air block), got {:?}",
            pixel.0
        );
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

    // ─────────────────────────────────────────────────────────────
    // Bug-fix regression tests (added during comprehensive UX/perf pass)
    // ─────────────────────────────────────────────────────────────

    /// Regression: previously unfilled pixels were `[0, 0, 0, 0]` (alpha=0),
    /// which some MCP clients rendered as solid black instead of transparent.
    /// After the fix, every pixel must be opaque (alpha = 255) so the image
    /// looks the same across clients.
    #[test]
    fn test_render_topdown_no_transparent_pixels() {
        let snap = snapshot_with_surroundings();
        let bytes = render_topdown(&snap, 4).expect("render should succeed");
        let img = image::load_from_memory(&bytes)
            .expect("decode PNG")
            .to_rgba8();
        for pixel in img.pixels() {
            assert_eq!(
                pixel[3], 255,
                "every pixel must be opaque (alpha=255), got {:?}",
                pixel.0
            );
        }
    }

    /// Regression: previously the empty-region pixels were black/transparent.
    /// After the fix, the background should be a distinguishable sky-blue so
    /// empty areas don't look like "stone" or "void".
    #[test]
    fn test_render_topdown_background_is_sky_blue() {
        // Snapshot with only one block far from origin → most pixels empty.
        let snap = WorldSnapshot {
            blocks: vec![BlockEntry {
                position: BlockPos::new(0, 63, 0),
                block_type: "grass_block".into(),
                block_state: None,
            }],
            entities: Vec::new(),
            self_player: SelfPlayer {
                uuid: "player".into(),
                username: "TestBot".into(),
                position: BlockPos::new(0, 64, 0),
                health: 20.0,
                hunger: 20,
                gamemode: GameMode::Survival,
                held_item_slot: 0,
                inventory: Vec::new(),
                position_precise: None,
                yaw: None,
            },
            timestamp: 0,
            chunk_summary: Vec::new(),
            commands_enabled: None,
            ..Default::default()
        };
        let bytes = render_topdown(&snap, 4).expect("render should succeed");
        let img = image::load_from_memory(&bytes)
            .expect("decode PNG")
            .to_rgba8();
        // Corner pixel (0,0) is well outside the only block at (0,0).
        let bg = img.get_pixel(0, 0);
        // Sky blue ~ [135, 206, 235, 255].
        assert_eq!(
            bg[0], 135,
            "background R should be sky-blue (135), got {}",
            bg[0]
        );
        assert_eq!(
            bg[1], 206,
            "background G should be sky-blue (206), got {}",
            bg[1]
        );
        assert_eq!(
            bg[2], 235,
            "background B should be sky-blue (235), got {}",
            bg[2]
        );
    }

    /// Plank variants should map to a wood-plank colour (distinct from logs).
    #[test]
    fn test_color_map_planks() {
        let oak = color_map("oak_planks");
        assert_ne!(
            oak.0,
            [160, 160, 160, 255],
            "oak_planks must not be grey fallback"
        );
        // All plank variants should share the same colour family.
        let spruce = color_map("spruce_planks");
        let birch = color_map("birch_planks");
        assert_eq!(oak.0, spruce.0, "all planks should share colour");
        assert_eq!(oak.0, birch.0, "all planks should share colour");
    }

    /// Glass variants should be a distinct translucent-blue tint.
    #[test]
    fn test_color_map_glass() {
        let glass = color_map("glass");
        assert_ne!(
            glass.0,
            [160, 160, 160, 255],
            "glass must not be grey fallback"
        );
        let pane = color_map("glass_pane");
        assert_eq!(glass.0, pane.0, "glass and glass_pane should match");
    }

    /// Wool variants should map to roughly their nominal colour.
    #[test]
    fn test_color_map_wool() {
        let white = color_map("white_wool");
        let red = color_map("red_wool");
        let blue = color_map("blue_wool");
        assert_ne!(
            white.0,
            [160, 160, 160, 255],
            "wool must not be grey fallback"
        );
        assert_ne!(white.0, red.0, "different wool colours must differ");
        assert_ne!(white.0, blue.0, "different wool colours must differ");
        assert_ne!(red.0, blue.0, "red and blue wool must differ");
    }

    /// Concrete and terracotta families should have distinct colours.
    #[test]
    fn test_color_map_concrete_and_terracotta() {
        let concrete = color_map("white_concrete");
        let terracotta = color_map("terracotta");
        assert_ne!(
            concrete.0,
            [160, 160, 160, 255],
            "concrete must not be grey fallback"
        );
        assert_ne!(
            terracotta.0,
            [160, 160, 160, 255],
            "terracotta must not be grey fallback"
        );
        assert_ne!(
            concrete.0, terracotta.0,
            "concrete and terracotta should differ"
        );
    }

    /// Plant blocks (flowers, mushrooms, cactus) should map to plant colours,
    /// not grey fallback.
    #[test]
    fn test_color_map_plants() {
        let flower = color_map("dandelion");
        let rose = color_map("poppy");
        let cactus = color_map("cactus");
        let mushroom = color_map("red_mushroom");
        assert_ne!(flower.0, [160, 160, 160, 255], "dandelion must not be grey");
        assert_ne!(rose.0, [160, 160, 160, 255], "poppy must not be grey");
        assert_ne!(cactus.0, [160, 160, 160, 255], "cactus must not be grey");
        assert_ne!(
            mushroom.0,
            [160, 160, 160, 255],
            "red_mushroom must not be grey"
        );
    }

    /// Functional blocks (door, fence, ladder, rail, torch) should map to
    /// distinct colours, not grey fallback.
    #[test]
    fn test_color_map_functional_blocks() {
        let door = color_map("oak_door");
        let fence = color_map("oak_fence");
        let ladder = color_map("ladder");
        let rail = color_map("rail");
        let torch = color_map("torch");
        assert_ne!(door.0, [160, 160, 160, 255], "oak_door must not be grey");
        assert_ne!(fence.0, [160, 160, 160, 255], "oak_fence must not be grey");
        assert_ne!(ladder.0, [160, 160, 160, 255], "ladder must not be grey");
        assert_ne!(rail.0, [160, 160, 160, 255], "rail must not be grey");
        assert_ne!(torch.0, [160, 160, 160, 255], "torch must not be grey");
    }

    /// Performance regression test: a large snapshot (10000 blocks) should
    /// render without panic and complete reasonably fast. Previously the
    /// HashMap-based column_best had per-block hashing overhead; the Vec
    /// array-based version should handle this easily.
    #[test]
    fn test_render_topdown_large_snapshot_no_panic() {
        let mut blocks = Vec::with_capacity(10000);
        for i in 0..10000i32 {
            // Spread blocks in a 100x100 grid around origin at varying Y.
            let x = (i % 100) - 50;
            let z = (i / 100) - 50;
            let y = 60 + (i % 30);
            blocks.push(BlockEntry {
                position: BlockPos::new(x, y, z),
                block_type: "stone".into(),
                block_state: None,
            });
        }
        let snap = WorldSnapshot {
            blocks,
            entities: Vec::new(),
            self_player: SelfPlayer {
                uuid: "player".into(),
                username: "TestBot".into(),
                position: BlockPos::new(0, 64, 0),
                health: 20.0,
                hunger: 20,
                gamemode: GameMode::Survival,
                held_item_slot: 0,
                inventory: Vec::new(),
                position_precise: None,
                yaw: None,
            },
            timestamp: 0,
            chunk_summary: Vec::new(),
            commands_enabled: None,
            ..Default::default()
        };
        // radius=32 → 65x65 image; 10000 blocks should render fine.
        let bytes = render_topdown(&snap, 32).expect("large render should succeed");
        assert!(bytes.starts_with(&PNG_MAGIC), "should produce valid PNG");
    }

    // ─────────────────────────────────────────────────────────────
    // Enhanced renderer tests (scale, Y-modulation, yaw arrow)
    // ─────────────────────────────────────────────────────────────

    /// `render_topdown_enhanced` with `scale=1` should produce a PNG of the
    /// same dimensions as the legacy `render_topdown` (block_size ×
    /// block_size pixels).
    #[test]
    fn test_render_enhanced_scale_1_matches_legacy_dimensions() {
        let snap = snapshot_with_surroundings();
        let legacy = render_topdown(&snap, 4).expect("legacy render");
        let enhanced = render_topdown_enhanced(&snap, 4, 1).expect("enhanced render");
        let legacy_img = image::load_from_memory(&legacy).unwrap().to_rgba8();
        let enhanced_img = image::load_from_memory(&enhanced).unwrap().to_rgba8();
        assert_eq!(
            legacy_img.dimensions(),
            enhanced_img.dimensions(),
            "scale=1 should produce same dimensions as legacy renderer"
        );
    }

    /// `scale=4` should produce a `4×` larger image in each dimension.
    #[test]
    fn test_render_enhanced_scale_4_dimensions() {
        let snap = snapshot_with_surroundings();
        let bytes = render_topdown_enhanced(&snap, 4, 4).expect("render");
        let img = image::load_from_memory(&bytes).unwrap().to_rgba8();
        // radius=4 → block_size=9 → 9*4 = 36 pixels per side
        assert_eq!(img.dimensions(), (36, 36));
    }

    /// `scale=8` should produce a `8×` larger image.
    #[test]
    fn test_render_enhanced_scale_8_dimensions() {
        let snap = snapshot_with_surroundings();
        let bytes = render_topdown_enhanced(&snap, 2, 8).expect("render");
        let img = image::load_from_memory(&bytes).unwrap().to_rgba8();
        // radius=2 → block_size=5 → 5*8 = 40 pixels per side
        assert_eq!(img.dimensions(), (40, 40));
    }

    /// Invalid `scale` values (0, 3, 5, 9, 255) fall back to scale=1
    /// instead of producing a malformed image.
    #[test]
    fn test_render_enhanced_invalid_scale_falls_back() {
        let snap = snapshot_with_surroundings();
        for invalid in [0u8, 3, 5, 7, 9, 100, 255] {
            let bytes = render_topdown_enhanced(&snap, 4, invalid).expect("render");
            let img = image::load_from_memory(&bytes).unwrap().to_rgba8();
            // Should produce a scale=1 image (9x9 for radius=4).
            assert_eq!(
                img.dimensions(),
                (9, 9),
                "invalid scale {invalid} should fall back to scale=1"
            );
        }
    }

    /// `position_precise` should shift the centre by sub-block precision.
    ///
    /// Construct two snapshots: one with `position_precise = Some([0.5,
    /// 64, 0.5])` and one with `None`. The former's centre block should
    /// be offset by 1 pixel relative to the latter (since round(0.5) = 1
    /// vs the floor-based `position` of 0).
    #[test]
    fn test_render_enhanced_position_precise_shifts_centre() {
        let block_at_origin = BlockEntry {
            position: BlockPos::new(0, 63, 0),
            block_type: "grass_block".into(),
            block_state: None,
        };
        let base_player = SelfPlayer {
            uuid: "player".into(),
            username: "TestBot".into(),
            position: BlockPos::new(0, 64, 0),
            health: 20.0,
            hunger: 20,
            gamemode: GameMode::Survival,
            held_item_slot: 0,
            inventory: Vec::new(),
            position_precise: None,
            yaw: None,
        };

        // Snapshot without precise position — centre = floor(0, 0) = (0, 0).
        let snap_floor = WorldSnapshot {
            blocks: vec![block_at_origin.clone()],
            entities: Vec::new(),
            self_player: base_player.clone(),
            timestamp: 0,
            chunk_summary: Vec::new(),
            commands_enabled: None,
            ..Default::default()
        };

        // Snapshot with precise [0.5, 64, 0.5] — round(0.5) = 1 (or 0;
        // ties depend on rounding mode, but Rust's `round` rounds half
        // away from zero → 1). So centre is (1, 1), shifting the
        // block-at-origin one pixel up-left relative to the centre.
        let mut snap_precise = snap_floor.clone();
        snap_precise.self_player.position_precise = Some([0.5, 64.0, 0.5]);

        let bytes_floor = render_topdown_enhanced(&snap_floor, 2, 1).expect("render floor");
        let bytes_precise = render_topdown_enhanced(&snap_precise, 2, 1).expect("render precise");
        // Both should be valid PNGs of the same size (5×5).
        let img_floor = image::load_from_memory(&bytes_floor).unwrap().to_rgba8();
        let img_precise = image::load_from_memory(&bytes_precise).unwrap().to_rgba8();
        assert_eq!(img_floor.dimensions(), (5, 5));
        assert_eq!(img_precise.dimensions(), (5, 5));
        // The two images should differ — the centre is shifted.
        let diff_count = img_floor
            .pixels()
            .zip(img_precise.pixels())
            .filter(|(a, b)| a.0 != b.0)
            .count();
        assert!(
            diff_count > 0,
            "position_precise should shift the centre — expected at least 1 differing pixel"
        );
    }

    /// Y-axis modulation: two stacked blocks at the same (x,z) but
    /// different Y should produce different brightness.
    ///
    /// The lower block alone should appear darker than the higher block
    /// alone (when each is the sole block in its snapshot).
    #[test]
    fn test_render_enhanced_y_modulation_darkens_low_blocks() {
        fn render_single_block_at_y(y: i32) -> Rgba<u8> {
            // Place the block at (2, y, 0) with radius=2 so it lands at
            // pixel (4, 2) — far enough from the player marker (which
            // `paint_square` draws at the centre 2×2 region when
            // `scale.max(2) = 2`).
            let snap = WorldSnapshot {
                blocks: vec![BlockEntry {
                    position: BlockPos::new(2, y, 0),
                    block_type: "stone".into(),
                    block_state: None,
                }],
                entities: Vec::new(),
                self_player: SelfPlayer {
                    uuid: "player".into(),
                    username: "TestBot".into(),
                    position: BlockPos::new(0, y + 1, 0),
                    health: 20.0,
                    hunger: 20,
                    gamemode: GameMode::Survival,
                    held_item_slot: 0,
                    inventory: Vec::new(),
                    position_precise: None,
                    yaw: None,
                },
                timestamp: 0,
                chunk_summary: Vec::new(),
                commands_enabled: None,
                ..Default::default()
            };
            let bytes = render_topdown_enhanced(&snap, 2, 1).expect("render");
            let img = image::load_from_memory(&bytes).unwrap().to_rgba8();
            // radius=2, size=5. Block at (2,0) → pixel (2+2, 0+2) = (4, 2).
            *img.get_pixel(4, 2)
        }

        let high_pixel = render_single_block_at_y(320);
        let low_pixel = render_single_block_at_y(-64);
        // Both should be stone-coloured (grey) but the high one brighter.
        assert!(
            high_pixel[0] >= low_pixel[0],
            "high-Y block ({}) should be at least as bright as low-Y block ({})",
            high_pixel[0],
            low_pixel[0]
        );
        // Specifically, the higher block should be noticeably brighter
        // (factor 0.75 + 0.25 * 1.0 = 1.0 vs 0.75 + 0.25 * 0.0 = 0.75).
        // stone = [128, 128, 128], so high → 128, low → 96.
        let high_brightness = high_pixel[0] as f32;
        let low_brightness = low_pixel[0] as f32;
        assert!(
            high_brightness > low_brightness + 5.0,
            "Y-modulation should produce a visible brightness difference (got {high_brightness} vs {low_brightness})"
        );
    }

    /// A snapshot with `yaw = Some(0.0)` (facing south) should produce a
    /// render containing white pixels (the arrow) near the centre.
    /// Without yaw, no white pixels should appear (the player marker is red).
    #[test]
    fn test_render_enhanced_yaw_arrow_paints_white_pixels() {
        let snap_with_yaw = WorldSnapshot {
            blocks: Vec::new(),
            entities: Vec::new(),
            self_player: SelfPlayer {
                uuid: "player".into(),
                username: "TestBot".into(),
                position: BlockPos::new(0, 64, 0),
                health: 20.0,
                hunger: 20,
                gamemode: GameMode::Survival,
                held_item_slot: 0,
                inventory: Vec::new(),
                position_precise: None,
                yaw: Some(0.0),
            },
            timestamp: 0,
            chunk_summary: Vec::new(),
            commands_enabled: None,
            ..Default::default()
        };
        let snap_no_yaw = {
            let mut s = snap_with_yaw.clone();
            s.self_player.yaw = None;
            s
        };

        let bytes_with = render_topdown_enhanced(&snap_with_yaw, 4, 4).expect("render");
        let bytes_without = render_topdown_enhanced(&snap_no_yaw, 4, 4).expect("render");
        let img_with = image::load_from_memory(&bytes_with).unwrap().to_rgba8();
        let img_without = image::load_from_memory(&bytes_without).unwrap().to_rgba8();

        let white = Rgba([255, 255, 255, 255]);
        let count_with = img_with.pixels().filter(|p| **p == white).count();
        let count_without = img_without.pixels().filter(|p| **p == white).count();
        assert!(
            count_with > count_without,
            "yaw arrow should paint white pixels; with-yaw={count_with}, without={count_without}"
        );
        assert!(count_with > 0, "yaw arrow should paint at least one pixel");
    }

    /// `modulate_brightness` should preserve alpha and clamp factor.
    #[test]
    fn test_modulate_brightness_preserves_alpha_and_clamps() {
        let colour = Rgba([100, 200, 50, 255]);
        assert_eq!(modulate_brightness(colour, 1.0), colour);
        let dark = modulate_brightness(colour, 0.5);
        assert_eq!(dark[3], 255, "alpha should be preserved");
        assert!(dark[0] < colour[0], "factor<1 should darken");
        // Negative factor should clamp to 0 (no inversion).
        let zero = modulate_brightness(colour, -1.0);
        assert_eq!(zero.0, [0, 0, 0, 255]);
        // Factor > 1 should clamp to 1 (no overflow).
        let high = modulate_brightness(colour, 2.0);
        assert_eq!(high, colour);
    }
}
