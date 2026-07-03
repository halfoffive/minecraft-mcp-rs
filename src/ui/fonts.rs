//! Platform-default CJK system font detection and egui installation.
//!
//! # Why this exists
//!
//! egui ships with a small set of Latin-only fonts (Ubuntu-Light, Hack,
//! NotoEmoji).  Without an extra CJK font, Simplified Chinese characters
//! render as empty boxes (tofu).  Rather than bundling a multi-megabyte
//! CJK font into the binary (which would bloat the release artifact), we
//! detect the platform's pre-installed system CJK font at runtime and
//! inject it into the egui [`FontDefinitions`].
//!
//! # Detection strategy
//!
//! [`candidate_font_paths`] returns a hard-coded list of well-known CJK
//! font locations per OS (Windows / macOS / Linux).  [`first_existing_font`]
//! walks that list and returns the first path that exists on disk.  The
//! chosen file is loaded with [`std::fs::read`] and registered under the
//! name `"cjk"` in the egui [`FontDefinitions`]:
//!
//! - Inserted at the **front** of `FontFamily::Proportional` so Chinese
//!   glyphs render with the system font while Latin glyphs still prefer
//!   egui's Ubuntu-Light (because the system CJK fonts also include Latin
//!   glyphs, the visual difference is minimal — and crucially, characters
//!   that only exist in the CJK font now resolve).
//! - Appended to the **back** of `FontFamily::Monospace` so monospace
//!   Latin text keeps using Hack, with the CJK font as a fallback for
//!   any non-Latin characters.
//!
//! If no candidate path exists (e.g. a stripped-down container) or the
//! file cannot be read, we log a warning and let egui fall back to its
//! default fonts — the UI still works, just with tofu instead of Chinese
//! characters.

use std::path::{Path, PathBuf};
use std::sync::Arc;

// ════════════════════════════════════════════════════════════════════
// Candidate path enumeration
// ════════════════════════════════════════════════════════════════════

/// Return platform-specific candidate paths for the system CJK font.
///
/// The list is ordered by preference: the first existing path wins.
/// Evaluated at runtime via `cfg!` (not `#[cfg]`) so the function compiles
/// on every platform and the candidates for the *current* OS are returned
/// — useful for tests that just want a non-empty list regardless of host.
///
/// # Platform coverage
///
/// - **Windows** — Microsoft YaHei (`msyh.ttc` / `msyh.ttf`) is the modern
///   default since Windows 7; SimSun (`simsun.ttc`) is a legacy fallback.
/// - **macOS** — PingFang SC has been the system CJK font since OS X 10.11
///   El Capitan; Arial Unicode is a classic fallback that includes CJK.
/// - **Linux** — Noto Sans CJK is the most common distribution default;
///   WenQuanYi Micro Hei is a lightweight fallback common on minimal
///   installs.
fn candidate_font_paths() -> Vec<&'static str> {
    if cfg!(windows) {
        vec![
            "C:\\Windows\\Fonts\\msyh.ttc",
            "C:\\Windows\\Fonts\\msyh.ttf",
            "C:\\Windows\\Fonts\\simsun.ttc",
        ]
    } else if cfg!(target_os = "macos") {
        vec![
            "/System/Library/Fonts/PingFang.ttc",
            "/Library/Fonts/Arial Unicode.ttf",
        ]
    } else {
        // Treat everything else as Linux-like.  This is the right default
        // for the production Linux desktop deployment target; other Unixes
        // (BSDs etc.) are unlikely hosts for this app.
        vec![
            "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
            "/usr/share/fonts/wqy-microhei/wqy-microhei.ttc",
            "/usr/share/fonts/wenquanyi/wqy-microhei/wqy-microhei.ttc",
        ]
    }
}

// ════════════════════════════════════════════════════════════════════
// First-existing-path resolution
// ════════════════════════════════════════════════════════════════════

/// Return the first candidate CJK font path that exists on disk.
///
/// Uses [`Path::try_exists`] (stable since Rust 1.63) so missing paths and
/// permission errors are both treated as "not present" without panicking.
/// Logs at `debug` level: the chosen path on success, or the full list of
/// checked paths on failure.
fn first_existing_font() -> Option<PathBuf> {
    let candidates = candidate_font_paths();
    for path in &candidates {
        // `try_exists` distinguishes "definitely does not exist" from
        // "couldn't tell" (e.g. permission denied).  For our purposes both
        // mean "skip this candidate" — and we unwrap_or(false) to be
        // defensive against exotic filesystem errors.
        if Path::new(path).try_exists().unwrap_or(false) {
            tracing::debug!(path = %path, "found CJK system font");
            return Some(PathBuf::from(path));
        }
    }
    tracing::debug!(checked = ?candidates, "no CJK system font candidate existed");
    None
}

// ════════════════════════════════════════════════════════════════════
// Public installer — wires the loaded font into egui
// ════════════════════════════════════════════════════════════════════

/// Detect, load, and install the platform-default CJK font into `ctx`.
///
/// Call this once during egui setup (see `main.rs`'s `CreationContext`
/// closure) so Simplified Chinese characters render correctly.
///
/// Behaviour:
/// - If a candidate font file exists and can be read, register it under
///   the name `"cjk"` in [`egui::FontDefinitions`], prioritised for the
///   `Proportional` family and appended as a fallback for `Monospace`.
/// - If no candidate exists, log a `warn` and leave egui's default fonts
///   in place (Chinese characters will show as tofu, but the UI remains
///   functional).
/// - If a candidate exists but cannot be read (permissions, I/O error),
///   log a `warn` with the error and likewise fall back.
///
/// This function is intentionally infallible: a font failure must never
/// crash the UI.
pub fn install_system_cjk_fonts(ctx: &egui::Context) {
    let Some(path) = first_existing_font() else {
        tracing::warn!("no CJK system font found; Chinese text may not render correctly");
        return;
    };

    // Read the font file.  A `.ttc` (TrueType Collection) is loaded the
    // same way as a single `.ttf` — egui / epaint picks face index 0 by
    // default, which is the regular weight for all the candidate fonts.
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::warn!(error = %e, path = %path.display(), "failed to load CJK font, falling back to default");
            return;
        }
    };

    // Build a fresh FontDefinitions from egui defaults, then splice in the
    // CJK font.  We wrap `FontData::from_owned` in `Arc::new` because egui
    // 0.34's `font_data` map is `BTreeMap<String, Arc<FontData>>`.
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "cjk".to_owned(),
        Arc::new(egui::FontData::from_owned(bytes)),
    );

    // Put "cjk" first in Proportional so any non-Latin glyph resolves to
    // the system CJK font; Latin glyphs still render fine because the CJK
    // fonts include Latin coverage.
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, "cjk".to_owned());

    // Append "cjk" as the last resort for Monospace so Latin code-style
    // text keeps using Hack/NotoEmoji and only falls back to CJK when a
    // glyph is missing from the primary monospace stack.
    fonts
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default()
        .push("cjk".to_owned());

    ctx.set_fonts(fonts);
    tracing::info!(path = %path.display(), "installed CJK system font");
}

// ════════════════════════════════════════════════════════════════════
// Tests
// ════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    /// `candidate_font_paths` returns a non-empty list on every platform
    /// the crate supports (Windows / macOS / Linux).  This guards against
    /// accidentally introducing a `cfg!` branch that returns `vec![]`.
    #[test]
    fn test_candidate_font_paths_nonempty() {
        assert!(
            !candidate_font_paths().is_empty(),
            "candidate_font_paths must return at least one path on every platform"
        );
    }

    /// `first_existing_font` must not panic even when none of the candidate
    /// paths exist (e.g. running inside a minimal CI container without CJK
    /// fonts installed).  We don't assert on the `Option` value because the
    /// result depends on the host filesystem; we just want no panic.
    #[test]
    fn test_first_existing_font_does_not_panic() {
        let _ = first_existing_font();
    }

    /// A deliberately-missing path resolves to `false` from `try_exists`,
    /// confirming the predicate we use to filter candidates actually
    /// rejects non-existent paths (rather than silently accepting them).
    #[test]
    fn test_first_existing_returns_none_for_missing() {
        let missing = Path::new("/nonexistent/definitely/not/here.ttc");
        assert!(!missing.try_exists().unwrap_or(false));
    }
}
