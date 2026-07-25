//! Screenshot preview panel: shows the most recent `get_world_view` render.
//!
//! The panel reads [`SharedState::get_world_view_cache`] each frame and
//! decodes the cached base64 PNG into an [`egui::ColorImage`] for display.
//! A "Refresh" button clears the cache and re-renders the current snapshot
//! at a fixed `radius=8, scale=2` (68×68 blocks → 136×136 pixels) — large
//! enough to be useful, small enough to decode every frame without lag.
//!
//! When the cache is empty (no `get_world_view` call has happened yet, or
//! the bot is offline and the cache was cleared), the panel shows a
//! placeholder message instead.

use base64::Engine;
use egui::{ColorImage, Image, TextureHandle, Ui};
use std::sync::Arc;

use crate::state::SharedState;
use crate::ui::i18n::{self, TextKey};

/// Render the world-view preview panel.
///
/// Shows:
/// - The cached PNG (decoded to a texture) if present.
/// - A "Refresh" button that clears the cache and re-renders the current
///   snapshot at `radius=8, scale=2`.
/// - A placeholder message when no render is cached or the bot is offline.
///
/// `texture` is a pass-through `Option<TextureHandle>` that the caller
/// owns across frames — egui textures must be retained between frames to
/// avoid re-uploading the same PNG every redraw. The caller passes a
/// mutable `Option<TextureHandle>` and this function updates it only when
/// the cached PNG's annotation JSON changes (so the texture is not
/// rebuilt every frame).
pub fn preview_panel(
    ui: &mut Ui,
    state: &Arc<SharedState>,
    texture: &mut Option<TextureHandle>,
    last_annotation: &mut Option<String>,
) {
    let cache = state.get_world_view_cache();

    // Rebuild the texture only when the annotation JSON changes (i.e. the
    // cached render actually changed). This avoids re-decoding the PNG
    // every frame, which would be a noticeable CPU hit at scale=8.
    let need_rebuild = match (&cache, &*last_annotation) {
        (Some(c), Some(prev)) => c.annotation_json != *prev,
        (Some(_), None) => true,
        (None, _) => false,
    };

    if need_rebuild {
        if let Some(ref c) = cache {
            match decode_png_to_texture(ui.ctx(), &c.png_base64) {
                Ok(handle) => {
                    *texture = Some(handle);
                    *last_annotation = Some(c.annotation_json.clone());
                }
                Err(e) => {
                    ui.colored_label(
                        egui::Color32::RED,
                        format!("{} {}", i18n::tr(TextKey::Error), e),
                    );
                    *texture = None;
                    *last_annotation = None;
                    state.clear_world_view_cache();
                }
            }
        } else {
            *texture = None;
            *last_annotation = None;
        }
    }

    // Header row: label + refresh button.
    ui.horizontal(|ui| {
        ui.label(i18n::tr(TextKey::WorldView));
        let is_online = state.is_online();
        let refresh_btn = ui
            .add_enabled(is_online, egui::Button::new(i18n::tr(TextKey::Refresh)))
            .on_hover_text(i18n::tr(TextKey::RefreshTooltip));
        if refresh_btn.clicked() {
            refresh_render(state);
        }
    });

    ui.separator();

    // Render the texture if present; otherwise show a placeholder.
    if let Some(ref tex) = *texture {
        // Constrain the image to fit within the available width so it
        // doesn't overflow the panel for large renders.
        let avail = ui.available_width().min(512.0);
        ui.add(Image::new(tex).max_width(avail));
        // Show the annotation below the image so the user can see the
        // centre coords / radius / scale / yaw without inspecting the
        // MCP response.
        if let Some(ref ann) = *last_annotation {
            ui.monospace(ann);
        }
    } else if state.is_online() {
        ui.label(i18n::tr(TextKey::WorldViewPlaceholder));
    } else {
        ui.colored_label(
            egui::Color32::from_rgb(150, 150, 150),
            i18n::tr(TextKey::Offline),
        );
    }
}

/// Force a fresh re-render of the current snapshot and update the cache.
///
/// Clears the cache first so the next `get_world_view` call is forced to
/// re-run the renderer (instead of returning a stale cache hit), then
/// calls the MCP-layer `get_world_view` with fixed `radius=8, scale=2`
/// (68×68 blocks → 136×136 pixels). The result is discarded — the side
/// effect (cache populated) is what we want.
fn refresh_render(state: &Arc<SharedState>) {
    state.clear_world_view_cache();
    // Call into the MCP layer directly. We discard the returned contents;
    // the cache is now populated for the next preview_panel render.
    if let Err(e) = crate::mcp::tools_query::get_world_view(state, 8, 2) {
        tracing::warn!(error = %e, "preview refresh render failed");
        state.set_last_error(format!("Preview render failed: {e}"));
    }
}

/// Decode a base64-encoded PNG into an egui texture handle.
///
/// Returns an error message (not a BotError — this is a UI-only path) if
/// the base64 or PNG decoding fails. On success the caller receives a
/// ready-to-use `TextureHandle` that can be displayed via `Image::new`.
fn decode_png_to_texture(ctx: &egui::Context, png_base64: &str) -> Result<TextureHandle, String> {
    let png_bytes = base64::engine::general_purpose::STANDARD
        .decode(png_base64)
        .map_err(|e| format!("base64 decode failed: {e}"))?;
    let img = image::load_from_memory(&png_bytes)
        .map_err(|e| format!("PNG decode failed: {e}"))?
        .to_rgba8();
    let (w, h) = img.dimensions();
    let pixels = img.into_raw();
    let color_image = ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &pixels);
    Ok(ctx.load_texture("world_view_preview", color_image, Default::default()))
}

// ═══════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::types::{BlockEntry, BlockPos, GameMode, SelfPlayer, WorldSnapshot};
    use base64::Engine;

    /// `refresh_render` populates the cache so a subsequent
    /// `get_world_view_cache` returns `Some`.
    #[test]
    fn test_refresh_render_populates_cache() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        state.set_online(true);
        // Seed a minimal snapshot so the renderer has something to draw.
        let snap = WorldSnapshot {
            blocks: vec![BlockEntry {
                position: BlockPos::new(0, 63, 0),
                block_type: "stone".into(),
                block_state: None,
            }],
            entities: Vec::new(),
            self_player: SelfPlayer {
                uuid: "p".into(),
                username: "Bot".into(),
                position: BlockPos::new(0, 64, 0),
                health: 20.0,
                hunger: 20,
                gamemode: GameMode::Survival,
                held_item_slot: 0,
                inventory: Vec::new(),
                position_precise: None,
                yaw: None,
            },
            timestamp: 1,
            chunk_summary: Vec::new(),
            commands_enabled: None,
            ..Default::default()
        };
        state.update_snapshot(snap);

        // Cache should be empty initially.
        assert!(state.get_world_view_cache().is_none());

        refresh_render(&state);

        let cache = state
            .get_world_view_cache()
            .expect("cache should be populated after refresh_render");
        assert_eq!(cache.radius, 8);
        assert_eq!(cache.scale, 2);
        assert!(!cache.png_base64.is_empty());
        // Decoded base64 should be a valid PNG.
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&cache.png_base64)
            .unwrap();
        assert!(decoded.starts_with(&[0x89, 0x50, 0x4E, 0x47]));
    }

    /// `refresh_render` when the bot is offline should not panic and
    /// should leave the cache empty (and set last_error).
    #[test]
    fn test_refresh_render_offline_does_not_panic() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        // Bot is offline (default).
        assert!(state.get_world_view_cache().is_none());
        refresh_render(&state);
        // Cache should still be empty.
        assert!(state.get_world_view_cache().is_none());
        // And last_error should be set.
        assert!(state.last_error().is_some());
    }
}
