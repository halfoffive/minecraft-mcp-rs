//! Common utility functions shared across modules.
//!
//! Currently hosts the `to_snake_case` helper used by the bot command
//! executor and snapshot updater to convert azalea registry Debug names
//! (e.g. `IronPickaxe`) into the snake_case item ids used by the
//! block/tool tables (`iron_pickaxe`), plus the `normalize_yaw` helper that
//! folds the player's raw look angle into Minecraft's `[-180, 180)` range
//! before it reaches the snapshot and the MCP `get_bot_status` /
//! `get_world_view` payloads.

use std::time::{Duration, Instant};

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

/// Case-insensitive ASCII substring search (no allocations at all).
///
/// Both `haystack` and `needle` are scanned byte-wise with a
/// non-allocating ASCII-folding comparison — the needle does NOT need to be
/// pre-lowercased (an earlier revision of this doc claimed it did; the
/// implementation folds both sides, and no caller pre-lowercases it).
/// Filtering thousands of blocks by `block_type` therefore allocates
/// nothing (see `get_nearby_blocks`). Non-ASCII bytes are compared
/// verbatim — block/item ids are pure ASCII, so this is safe for them; for
/// non-ASCII input the comparison is byte-exact rather than
/// case-insensitive.
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

/// Backdate an `Instant` by `ago`, saturating at the clock origin.
///
/// `Instant - Duration` is a *panicking* subtraction: the Linux/macOS
/// monotonic clocks are boot-relative, so backdating 3600 s on a machine
/// with less than one hour of uptime panics — `BotState::default` used to
/// kill the bot connection thread exactly there (2026-08-30 review P1).
/// Callers use the backdate to make a throttle fire on the first tick; when
/// the backdate cannot be represented, the `Instant::now()` fallback merely
/// delays the first build by one throttle interval instead of panicking.
pub(crate) fn backdate_instant(ago: Duration) -> Instant {
    Instant::now().checked_sub(ago).unwrap_or_else(Instant::now)
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
///
/// Non-finite input is out of contract here: NaN fails both comparisons and
/// propagates verbatim. Production callers must go through
/// [`normalize_yaw_checked`], which maps non-finite angles to `None`; this
/// function keeps the total-`f32 -> f32` shape so tests can pin the exact
/// fold arithmetic.
pub fn normalize_yaw(yaw: f32) -> f32 {
    let degrees = yaw.to_degrees().rem_euclid(360.0);
    if degrees >= 180.0 {
        degrees - 360.0
    } else {
        degrees
    }
}

/// [`normalize_yaw`] with a non-finite guard: `NaN`/±∞ have no meaningful
/// direction, so they fold to `None` and the caller stores "yaw unknown"
/// (`SelfPlayer::yaw == None`, no renderer heading arrow) instead of a
/// value that would poison annotations (2026-08-26 review).
///
/// The snapshot updater's population of `SelfPlayer::yaw` is the single
/// production caller — one write point, like the unbounded-turn fix before
/// it.
pub fn normalize_yaw_checked(yaw: f32) -> Option<f32> {
    yaw.is_finite().then(|| normalize_yaw(yaw))
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

    // ── backdate_instant (2026-08-30 review P1) ──────────────────

    /// The backdate must land strictly before `now` when representable, and
    /// a raw `Instant - Duration` underflow (boot-relative clocks on a fresh
    /// Linux/macOS boot) must saturate instead of panicking. The underflow
    /// branch itself is time-of-day dependent — Windows `Instant` carries a
    /// large boot offset and can represent even the 100-year backdate — so
    /// the test asserts only the portable contract: never panics, never in
    /// the future, representable backdates move the point backwards.
    #[test]
    fn test_backdate_instant_saturates_without_panic() {
        let now = Instant::now();
        // A representable backdate lands strictly before now.
        let back = backdate_instant(Duration::from_secs(3600));
        assert!(back < now, "3600 s backdate must be in the past");
        // A deliberately huge backdate would panic a raw subtraction on
        // boot-relative monotonic clocks; here it must saturate to ~now.
        let century = backdate_instant(Duration::from_secs(60 * 60 * 24 * 365 * 100));
        assert!(
            century <= Instant::now(),
            "saturated backdate is never in the future"
        );
        // Zero backdate is "now" (within scheduler jitter).
        assert!(backdate_instant(Duration::ZERO) <= Instant::now());
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

    #[test]
    fn test_normalize_yaw_checked_rejects_non_finite() {
        // NaN/±∞ carry no direction: the checked wrapper folds them to None
        // so SelfPlayer::yaw stays "unknown" instead of poisoning
        // annotations (2026-08-26 review). Finite values pass through
        // unchanged.
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(
                normalize_yaw_checked(bad).is_none(),
                "non-finite {bad} must fold to None"
            );
        }
        let yaw = normalize_yaw_checked((-767.1_f32).to_radians()).expect("finite → Some");
        assert_yaw_eq(yaw, -47.1);
        assert!(normalize_yaw_checked(0.0).is_some());
    }
}
