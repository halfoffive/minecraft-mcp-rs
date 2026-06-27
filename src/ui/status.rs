//! Status panel: connection state, player info, nearby stats, chat log.
//!
//! Reads lock-free from [`SharedState`] via [`ArcSwap::load`](arc_swap::ArcSwap::load)
//! for the world snapshot, and uses short-lived read locks for config and stats.
//!
//! All user-visible strings are translated via the
//! [`i18n`](crate::ui::i18n) layer.

use egui::Ui;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::state::SharedState;
use crate::ui::i18n::{self, TextKey};

/// Render the status panel.
///
/// Displays:
/// - Last error message (red banner, only when present)
/// - Connection status (online/offline with uptime)
/// - Player information (position, health, hunger, gamemode)
/// - World stats (blocks, entities, chunks loaded)
/// - Command counters (processed, succeeded, failed)
/// - Last 10 chat messages
pub fn status_panel(ui: &mut Ui, state: &Arc<SharedState>) {
    let is_online = state.is_online();
    let snapshot = state.read_snapshot();
    // Read connected_since under the lock, then drop the guard immediately.
    // The atomic counters (commands_processed etc.) don't need the lock.
    let connected_since = state.read_run_stats().connected_since;
    let chat = state.get_chat_messages();

    // ── Last Error ────────────────────────────────────────────────────
    // Display a prominent red banner if the bot/MCP layer has reported an
    // error.  When there is no error, nothing is rendered (no empty row).
    if let Some(msg) = state.last_error() {
        ui.colored_label(
            egui::Color32::RED,
            format!("{} {}", i18n::tr(TextKey::Error), msg),
        );
        ui.separator();
    }

    // ── Connection ────────────────────────────────────────────────────

    ui.horizontal(|ui| {
        ui.label(i18n::tr(TextKey::Connection));
        if is_online {
            ui.colored_label(egui::Color32::GREEN, i18n::tr(TextKey::Online));
        } else {
            ui.colored_label(egui::Color32::RED, i18n::tr(TextKey::Offline));
        }
    });

    if let Some(since) = connected_since {
        let elapsed = since.elapsed();
        ui.label(format!(
            "{} {}{}",
            i18n::tr(TextKey::Uptime),
            elapsed.as_secs(),
            i18n::tr(TextKey::UnitSeconds)
        ));
    }

    ui.separator();

    // ── Player Info ───────────────────────────────────────────────────

    ui.collapsing(i18n::tr(TextKey::PlayerInfo), |ui| {
        let player = &snapshot.self_player;
        ui.label(format!(
            "{} {}",
            i18n::tr(TextKey::Username),
            player.username
        ));
        ui.label(format!("{} {}", i18n::tr(TextKey::Uuid), player.uuid));
        ui.label(format!(
            "{} ({}, {}, {})",
            i18n::tr(TextKey::Position),
            player.position.x,
            player.position.y,
            player.position.z
        ));
        ui.label(format!(
            "{} {:.1} / 20.0",
            i18n::tr(TextKey::Health),
            player.health
        ));
        ui.label(format!(
            "{} {} / 20",
            i18n::tr(TextKey::Hunger),
            player.hunger
        ));
        ui.label(format!(
            "{} {:?}",
            i18n::tr(TextKey::Gamemode),
            player.gamemode
        ));
        ui.label(format!(
            "{} {}",
            i18n::tr(TextKey::HeldSlot),
            player.held_item_slot
        ));
    });

    ui.separator();

    // ── Nearby Stats ──────────────────────────────────────────────────

    ui.collapsing(i18n::tr(TextKey::NearbyStats), |ui| {
        ui.label(format!(
            "{} {}",
            i18n::tr(TextKey::BlocksInView),
            snapshot.blocks.len()
        ));
        ui.label(format!(
            "{} {}",
            i18n::tr(TextKey::EntitiesInView),
            snapshot.entities.len()
        ));
        ui.label(format!(
            "{} {}",
            i18n::tr(TextKey::ChunksLoaded),
            snapshot.chunk_summary.len()
        ));

        if !snapshot.chunk_summary.is_empty() {
            ui.label(i18n::tr(TextKey::LoadedChunks));
            for (cx, cz) in &snapshot.chunk_summary {
                ui.label(format!("  {} ({}, {})", i18n::tr(TextKey::Chunk), cx, cz));
            }
        }
    });

    ui.separator();

    // ── Command Stats ─────────────────────────────────────────────────

    ui.collapsing(i18n::tr(TextKey::CommandStats), |ui| {
        // Re-acquire the stats guard only for this section. The atomic
        // counters are read through the guard; the lock is released when
        // the section ends.
        let stats = state.read_run_stats();
        let processed = stats.commands_processed.load(Ordering::Relaxed);
        let succeeded = stats.commands_succeeded.load(Ordering::Relaxed);
        let failed = stats.commands_failed.load(Ordering::Relaxed);

        ui.label(format!(
            "{} {}",
            i18n::tr(TextKey::CommandsProcessed),
            processed
        ));
        ui.label(format!("{} {}", i18n::tr(TextKey::Succeeded), succeeded));
        ui.label(format!("{} {}", i18n::tr(TextKey::Failed), failed));

        if processed > 0 {
            let rate = (succeeded as f64 / processed as f64) * 100.0;
            ui.label(format!("{} {:.1}%", i18n::tr(TextKey::SuccessRate), rate));
        }
    });

    ui.separator();

    // ── Chat Log ──────────────────────────────────────────────────────

    ui.collapsing(i18n::tr(TextKey::ChatLog), |ui| {
        if chat.is_empty() {
            ui.label(i18n::tr(TextKey::NoChatMessages));
        } else {
            egui::ScrollArea::vertical()
                .max_height(200.0)
                .show(ui, |ui| {
                    for (sender, message) in &chat {
                        // Chat line format "<sender> message" is universal
                        // across languages; left untranslated on purpose.
                        ui.monospace(format!("<{sender}> {message}"));
                    }
                });
        }
    });
}
