//! Screenshot preview panel: shows the most recent `get_world_view` render.
//!
//! The panel reads [`SharedState::world_view_cache_meta`] each frame — a
//! cheap accessor that never clones the cached base64 PNG (M-11) — and
//! decodes the full cached PNG into an [`egui::ColorImage`] only when a
//! rebuild is genuinely needed. A "Refresh" button clears the cache and
//! re-renders the current snapshot at a fixed `radius=8, scale=2`
//! (17×17 blocks → 34×34 pixels) — small enough to decode without lag.
//!
//! When the cache is empty (no `get_world_view` call has happened yet, or
//! the bot is offline and the cache was cleared), the panel shows a
//! placeholder message instead.

use base64::Engine;
use egui::{ColorImage, Image, TextureHandle, Ui};
use std::sync::Arc;

use crate::i18n::{self, TextKey};
use crate::state::{SharedState, WorldViewCacheMeta};

/// Rebuild key for the preview texture: the `(snapshot_seq, annotation)`
/// pair the texture was last built from (M-11).
///
/// Two snapshot builds inside the same wall-clock second share a
/// seconds-granular timestamp, so their annotation JSON can be identical
/// while the underlying PNG differs (block types changed). Keying on
/// [`WorldViewCacheMeta::snapshot_seq`] in addition to the annotation
/// catches that case — the same fix W-2 already applied to the MCP-layer
/// cache.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PreviewRebuildKey {
    /// `WorldSnapshot::snapshot_seq` of the snapshot the texture was
    /// rendered from.
    pub snapshot_seq: u64,
    /// The annotation JSON the texture was rendered with (also shown below
    /// the image so the user can see centre coords / radius / scale / yaw).
    pub annotation_json: String,
}

/// Decide whether the preview texture must be rebuilt this frame.
///
/// `meta` is the cheap metadata of the current world-view cache entry;
/// `last_key` is the key the texture was last built from.
///
/// - `(Some(m), Some(prev))` → rebuild only when `snapshot_seq` or the
///   annotation changed.
/// - `(Some(_), None)` → first render — build.
/// - `(None, _)` → no cache (nothing rendered yet, or the bot went offline
///   and `handle_disconnect` cleared the cache) — clear the stale texture
///   so the panel does not keep showing a frozen frame after a disconnect.
fn should_rebuild(meta: &Option<WorldViewCacheMeta>, last_key: &Option<PreviewRebuildKey>) -> bool {
    match (meta, last_key) {
        (Some(m), Some(prev)) => {
            m.snapshot_seq != prev.snapshot_seq || m.annotation_json != prev.annotation_json
        }
        (Some(_), None) => true,
        (None, _) => true,
    }
}

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
/// the cached render's `(snapshot_seq, annotation)` key changes (so the
/// texture is not rebuilt every frame).
pub fn preview_panel(
    ui: &mut Ui,
    state: &Arc<SharedState>,
    texture: &mut Option<TextureHandle>,
    last_key: &mut Option<PreviewRebuildKey>,
) {
    // Each frame read ONLY the cheap cache meta — never the whole
    // `WorldViewCache` with its ~700 KB base64 PNG (M-11).
    let meta = state.world_view_cache_meta();

    // Rebuild the texture only when the cached render actually changed
    // (snapshot_seq and/or annotation JSON differs) or the cache is gone
    // (first render, or a disconnect cleared it — a stale frame must not
    // linger). The full cache — including the PNG — is fetched only here,
    // when a rebuild is genuinely needed.
    if should_rebuild(&meta, last_key) {
        let cache = state.get_world_view_cache();
        if let Some(ref c) = cache {
            match decode_png_to_texture(ui.ctx(), &c.png_base64) {
                Ok(handle) => {
                    *texture = Some(handle);
                    *last_key = Some(PreviewRebuildKey {
                        snapshot_seq: c.snapshot_seq,
                        annotation_json: c.annotation_json.clone(),
                    });
                }
                Err(e) => {
                    ui.colored_label(
                        egui::Color32::RED,
                        format!("{} {}", i18n::tr(TextKey::Error), e),
                    );
                    *texture = None;
                    *last_key = None;
                    state.clear_world_view_cache();
                }
            }
        } else {
            *texture = None;
            *last_key = None;
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
        if let Some(ref key) = *last_key {
            ui.monospace(&key.annotation_json);
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
/// (17×17 blocks → 34×34 pixels). The result is discarded — the side
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

    // ── should_rebuild (M-11) ─────────────────────────────────────────

    fn meta(seq: u64, annotation: &str) -> crate::state::WorldViewCacheMeta {
        crate::state::WorldViewCacheMeta {
            snapshot_seq: seq,
            radius: 8,
            scale: 2,
            annotation_json: annotation.into(),
        }
    }

    fn key(seq: u64, annotation: &str) -> PreviewRebuildKey {
        PreviewRebuildKey {
            snapshot_seq: seq,
            annotation_json: annotation.into(),
        }
    }

    /// A missing cache always forces a rebuild/clear — the very first frame
    /// must build the texture, and a disconnect-cleared cache must drop the
    /// stale texture instead of keeping a frozen frame.
    #[test]
    fn test_should_rebuild_missing_cache_always_rebuilds() {
        let meta: Option<crate::state::WorldViewCacheMeta> = None;
        assert!(should_rebuild(&meta, &None));
        assert!(should_rebuild(&meta, &Some(key(1, "old annotation"))));
    }

    /// A fresh cache entry with no previous key must be rendered.
    #[test]
    fn test_should_rebuild_first_render() {
        assert!(should_rebuild(&Some(meta(1, "ann")), &None));
    }

    /// An unchanged (seq, annotation) pair is a cache hit — no rebuild.
    #[test]
    fn test_should_rebuild_unchanged_annotation_skips() {
        assert!(!should_rebuild(&Some(meta(1, "ann")), &Some(key(1, "ann"))));
    }

    /// A changed annotation forces a rebuild.
    #[test]
    fn test_should_rebuild_changed_annotation_rebuilds() {
        assert!(should_rebuild(
            &Some(meta(1, "new-ann")),
            &Some(key(1, "old-ann"))
        ));
    }

    /// M-11: two snapshot builds inside the same wall-clock second share a
    /// seconds-granular timestamp in the annotation, so the annotation can
    /// be identical while the PNG differs (block types changed). The rebuild
    /// decision must key on `snapshot_seq` in addition to the annotation.
    #[test]
    fn test_preview_rebuilds_on_seq_change_with_same_annotation() {
        assert!(
            should_rebuild(&Some(meta(2, "ann")), &Some(key(1, "ann"))),
            "same annotation but newer snapshot_seq must rebuild"
        );
    }
}
