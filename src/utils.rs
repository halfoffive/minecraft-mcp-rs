//! Common utility functions shared across modules.
//!
//! Currently hosts the `to_snake_case` helper used by the bot command
//! executor and snapshot updater to convert azalea registry Debug names
//! (e.g. `IronPickaxe`) into the snake_case item ids used by the
//! block/tool tables (`iron_pickaxe`), plus the `normalize_yaw` helper that
//! folds the player's raw look angle into Minecraft's `[-180, 180)` range
//! before it reaches the snapshot and the MCP `get_bot_status` /
//! `get_world_view` payloads.

/// Naive CamelCase → snake_case conversion.
///
/// Inserts `_` before each uppercase letter (except at the start) and
/// lowercases the result. Sufficient for azalea registry variant names.
///
/// **ASCII-only contract:** the input is an azalea registry name
/// (block/item/entity variant, e.g. `IronPickaxe`), which is pure ASCII by
/// construction. The uppercase detection uses Unicode-aware
/// [`is_uppercase`](char::is_uppercase) but the folding uses
/// [`to_ascii_lowercase`](char::to_ascii_lowercase), so a non-ASCII uppercase
/// character (e.g. `É`) would emit an underscore followed by the character
/// unchanged (not lowercased); non-ASCII lowercase passes through untouched.
/// This is latent — no azalea registry name contains non-ASCII — but the
/// behaviour is pinned by `test_to_snake_case_non_ascii_documented_behavior`
/// so an accidental caller feeding non-ASCII input gets defined output.
pub fn to_snake_case(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 4);
    for (i, ch) in s.chars().enumerate() {
        if ch.is_uppercase() {
            if i > 0 {
                result.push('_');
            }
            result.push(ch.to_ascii_lowercase());
        } else {
            result.push(ch);
        }
    }
    result
}

/// Case-insensitive ASCII substring search (single allocation for the
/// lowercased needle, none per haystack).
///
/// `needle` must already be lowercased. `haystack` is scanned byte-wise with
/// a non-allocating ASCII-folding comparison, so filtering thousands of
/// blocks by `block_type` no longer allocates one lowercase `String` per
/// block (see `get_nearby_blocks`). Non-ASCII bytes are compared verbatim —
/// block/item ids are pure ASCII, so this is safe for them; for non-ASCII
/// input the comparison is byte-exact rather than case-insensitive.
pub fn contains_ascii_case_insensitive(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let needle_bytes = needle.as_bytes();
    let hay_bytes = haystack.as_bytes();
    if needle_bytes.len() > hay_bytes.len() {
        return false;
    }
    let last_start = hay_bytes.len() - needle_bytes.len();
    (0..=last_start).any(|start| {
        hay_bytes[start..start + needle_bytes.len()]
            .iter()
            .zip(needle_bytes)
            .all(|(h, n)| h.eq_ignore_ascii_case(n))
    })
}

/// Fold a raw horizontal look angle (radians) into Minecraft's canonical
/// `[-180, 180)` degree range.
///
/// `LookDirection::y_rot()` can grow unboundedly as the player keeps turning
/// the same way (the functional test observed `yaw: -767.1` after several
/// spins), and a `-767.1` in the `get_bot_status` / `get_world_view`
/// annotation is meaningless to LLM clients. `rem_euclid(360)` maps any
/// value into `[0, 360)`, then values ≥ 180 are folded back into
/// `[-180, 180)` (so `270° → -90°`).
pub fn normalize_yaw(yaw: f32) -> f32 {
    let degrees = yaw.to_degrees().rem_euclid(360.0);
    if degrees >= 180.0 {
        degrees - 360.0
    } else {
        degrees
    }
}

// ═══════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_iron_pickaxe() {
        assert_eq!(to_snake_case("IronPickaxe"), "iron_pickaxe");
    }

    #[test]
    fn test_empty_string() {
        assert_eq!(to_snake_case(""), "");
    }

    #[test]
    fn test_all_uppercase() {
        assert_eq!(to_snake_case("ABC"), "a_b_c");
    }

    #[test]
    fn test_already_snake_case() {
        assert_eq!(to_snake_case("already_snake"), "already_snake");
    }

    #[test]
    fn test_single_char() {
        assert_eq!(to_snake_case("A"), "a");
        assert_eq!(to_snake_case("a"), "a");
    }

    #[test]
    fn test_consecutive_uppercase() {
        assert_eq!(to_snake_case("HTMLParser"), "h_t_m_l_parser");
    }

    #[test]
    fn test_common_registry_names() {
        assert_eq!(to_snake_case("GrassBlock"), "grass_block");
        assert_eq!(to_snake_case("Stone"), "stone");
        assert_eq!(to_snake_case("OakPlanks"), "oak_planks");
        assert_eq!(to_snake_case("DiamondOre"), "diamond_ore");
        assert_eq!(to_snake_case("NetheriteBlock"), "netherite_block");
    }

    /// Characterization test pinning the documented ASCII-only contract
    /// (L-10): `is_uppercase()` is Unicode-aware but `to_ascii_lowercase()`
    /// only folds ASCII. A non-ASCII uppercase at the start therefore passes
    /// through unchanged (no leading underscore, no case folding); a
    /// non-ASCII lowercase passes through untouched. Azalea registry names
    /// are pure ASCII, so this never fires in production — the test exists
    /// to freeze the behaviour if non-ASCII input ever reaches the helper.
    ///
    /// This is a characterization test: it passed immediately on the code it
    /// documents (no behaviour was changed by the fix — only the contract was
    /// written down).
    #[test]
    fn test_to_snake_case_non_ascii_documented_behavior() {
        // 'É' is uppercase but not ASCII: no underscore inserted at position
        // 0, and to_ascii_lowercase leaves it unchanged.
        assert_eq!(to_snake_case("Éclair"), "Éclair");
        // 'é' is lowercase: passes through verbatim; the ASCII 'L' still gets
        // the underscore + lowercase treatment.
        assert_eq!(to_snake_case("CaféLatte"), "café_latte");
    }

    // ── contains_ascii_case_insensitive (B5) ────────────────────────

    #[test]
    fn test_contains_ascii_case_insensitive_matches() {
        assert!(contains_ascii_case_insensitive("GrassBlock", "grass"));
        assert!(contains_ascii_case_insensitive("grass_block", "GRASS"));
        assert!(contains_ascii_case_insensitive("DIAMOND_ORE", "ore"));
        assert!(contains_ascii_case_insensitive("stone", "ST"));
    }

    #[test]
    fn test_contains_ascii_case_insensitive_no_match() {
        assert!(!contains_ascii_case_insensitive("grass_block", "dirt"));
        assert!(!contains_ascii_case_insensitive("stone", "nt")); // non-contiguous
        assert!(!contains_ascii_case_insensitive("ab", "abc")); // needle longer
    }

    #[test]
    fn test_contains_ascii_case_insensitive_empty_needle() {
        assert!(contains_ascii_case_insensitive("", ""));
        assert!(contains_ascii_case_insensitive("stone", ""));
    }

    #[test]
    fn test_contains_ascii_case_insensitive_multi_byte_verbatim() {
        // Non-ASCII bytes are compared verbatim (no case folding) — ids are
        // ASCII, but the helper must not panic or allocate on them.
        assert!(contains_ascii_case_insensitive("石stone", "stone"));
        assert!(!contains_ascii_case_insensitive("石", "石石"));
    }

    // ── normalize_yaw ─────────────────────────────────────────────

    /// Assert two angles are equal within a small f32 tolerance (the
    /// radian↔degree round-trip is not bit-exact).
    fn assert_yaw_eq(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 0.001,
            "expected {expected}°, got {actual}°"
        );
    }

    #[test]
    fn test_normalize_yaw_basic() {
        assert_yaw_eq(normalize_yaw(0.0), 0.0);
        assert_yaw_eq(normalize_yaw(90.0_f32.to_radians()), 90.0);
        assert_yaw_eq(normalize_yaw((-90.0_f32).to_radians()), -90.0);
        assert_yaw_eq(normalize_yaw(180.0_f32.to_radians()), -180.0);
        assert_yaw_eq(normalize_yaw(359.0_f32.to_radians()), -1.0);
    }

    #[test]
    fn test_normalize_yaw_wraps_accumulated_turns() {
        // Several full turns in one direction must collapse back into
        // [-180, 180) — the functional test observed -767.1°.
        assert_yaw_eq(normalize_yaw((-767.1_f32).to_radians()), -47.1);
        assert_yaw_eq(normalize_yaw(400.0_f32.to_radians()), 40.0);
        assert_yaw_eq(normalize_yaw(270.0_f32.to_radians()), -90.0);
        assert_yaw_eq(normalize_yaw((-270.0_f32).to_radians()), 90.0);
        assert_yaw_eq(normalize_yaw(720.0_f32.to_radians()), 0.0);
    }
}
