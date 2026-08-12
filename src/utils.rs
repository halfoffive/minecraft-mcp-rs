//! Common utility functions shared across modules.
//!
//! Currently hosts the `to_snake_case` helper used by the bot command
//! executor and snapshot updater to convert azalea registry Debug names
//! (e.g. `IronPickaxe`) into the snake_case item ids used by the
//! block/tool tables (`iron_pickaxe`).

/// Naive CamelCase → snake_case conversion.
///
/// Inserts `_` before each uppercase letter (except at the start) and
/// lowercases the result. Sufficient for azalea registry variant names.
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
}
