//! Application icon: embeds `assets/icon.png` and decodes it for the
//! egui window (taskbar / title bar / Alt-Tab).
//!
//! This is independent of the Windows executable icon embedded by
//! `build.rs` (the PE resource section) — that one covers Explorer,
//! shortcuts and the taskbar before the window exists; this module covers
//! the runtime window icon, which also benefits Linux (Wayland/X11) and
//! macOS where no resource compiler runs.
//!
//! Mirrors the CJK font loader's graceful-degradation contract: a decode
//! failure logs a warning and returns `None`, never panics — the app must
//! still start with the platform-default icon.

use tracing::warn;

// ══════════════════════════════════════════════════════════════════
// Embedded icon bytes
// ══════════════════════════════════════════════════════════════════

/// Raw bytes of `assets/icon.png` (512×512 RGBA), compiled into the binary.
pub const APP_ICON_PNG: &[u8] = include_bytes!("../../assets/icon.png");

// ══════════════════════════════════════════════════════════════════
// Public API
// ══════════════════════════════════════════════════════════════════

/// Decodes [`APP_ICON_PNG`] into an [`egui::IconData`] for
/// `egui::ViewportBuilder::with_icon`.
///
/// Returns `None` (with a `warn!`) when the PNG cannot be decoded — the
/// caller should then skip `.with_icon(...)` and let the window use the
/// platform default instead of failing startup over a cosmetic asset.
pub fn load_app_icon() -> Option<egui::IconData> {
    match eframe::icon_data::from_png_bytes(APP_ICON_PNG) {
        Ok(icon) => Some(icon),
        Err(e) => {
            warn!("failed to decode embedded app icon: {e}");
            None
        }
    }
}

// ══════════════════════════════════════════════════════════════════
// Tests
// ══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    /// Side length (in pixels) of the embedded square icon.
    const APP_ICON_SIZE_PX: u32 = 512;

    #[test]
    fn test_load_app_icon_decodes_embedded_png() {
        let icon = load_app_icon().expect("embedded icon must decode");
        assert_eq!(icon.width, APP_ICON_SIZE_PX);
        assert_eq!(icon.height, APP_ICON_SIZE_PX);
        // RGBA8: 4 bytes per pixel.
        assert_eq!(
            icon.rgba.len(),
            (APP_ICON_SIZE_PX * APP_ICON_SIZE_PX * 4) as usize
        );
    }

    #[test]
    fn test_app_icon_png_is_nonempty() {
        assert!(!APP_ICON_PNG.is_empty());
        // PNG magic number — guards against the asset being clobbered by a
        // non-PNG file without forcing an image-crate dependency here.
        assert_eq!(&APP_ICON_PNG[..8], b"\x89PNG\r\n\x1a\n");
    }
}
