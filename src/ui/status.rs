//! Status panel: connection state, player info, nearby stats, chat log.
//!
//! Reads lock-free from [`SharedState`] via [`ArcSwap::load`](arc_swap::ArcSwap::load)
//! for the world snapshot, and uses short-lived read locks for config and stats.

use egui::Ui;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::state::SharedState;

/// Render the status panel.
///
/// Displays:
/// - Connection status (online/offline with uptime)
/// - Player information (position, health, hunger, gamemode)
/// - World stats (blocks, entities, chunks loaded)
/// - Command counters (processed, succeeded, failed)
/// - Last 10 chat messages
pub fn status_panel(ui: &mut Ui, state: &Arc<SharedState>) {
    let is_online = state.is_online();
    let snapshot = state.read_snapshot();
    let stats = state.read_run_stats();
    let chat = state.get_chat_messages();

    // ── Connection ────────────────────────────────────────────────────

    ui.horizontal(|ui| {
        ui.label("Connection:");
        if is_online {
            ui.colored_label(egui::Color32::GREEN, "● Online");
        } else {
            ui.colored_label(egui::Color32::RED, "● Offline");
        }
    });

    if let Some(since) = stats.connected_since {
        let elapsed = since.elapsed();
        ui.label(format!("Uptime: {}s", elapsed.as_secs()));
    }

    ui.separator();

    // ── Player Info ───────────────────────────────────────────────────

    ui.collapsing("Player Info", |ui| {
        let player = &snapshot.self_player;
        ui.label(format!("Username: {}", player.username));
        ui.label(format!("UUID: {}", player.uuid));
        ui.label(format!(
            "Position: ({}, {}, {})",
            player.position.x, player.position.y, player.position.z
        ));
        ui.label(format!("Health: {:.1} / 20.0", player.health));
        ui.label(format!("Hunger: {} / 20", player.hunger));
        ui.label(format!("Gamemode: {:?}", player.gamemode));
        ui.label(format!("Held Slot: {}", player.held_item_slot));
    });

    ui.separator();

    // ── Nearby Stats ──────────────────────────────────────────────────

    ui.collapsing("Nearby Stats", |ui| {
        ui.label(format!("Blocks in view: {}", snapshot.blocks.len()));
        ui.label(format!("Entities in view: {}", snapshot.entities.len()));
        ui.label(format!("Chunks loaded: {}", snapshot.chunk_summary.len()));

        if !snapshot.chunk_summary.is_empty() {
            ui.label("Loaded chunks:");
            for (cx, cz) in &snapshot.chunk_summary {
                ui.label(format!("  chunk ({cx}, {cz})"));
            }
        }
    });

    ui.separator();

    // ── Command Stats ─────────────────────────────────────────────────

    ui.collapsing("Command Stats", |ui| {
        let processed = stats.commands_processed.load(Ordering::Relaxed);
        let succeeded = stats.commands_succeeded.load(Ordering::Relaxed);
        let failed = stats.commands_failed.load(Ordering::Relaxed);

        ui.label(format!("Commands processed: {processed}"));
        ui.label(format!("Succeeded: {succeeded}"));
        ui.label(format!("Failed: {failed}"));

        if processed > 0 {
            let rate = (succeeded as f64 / processed as f64) * 100.0;
            ui.label(format!("Success rate: {rate:.1}%"));
        }
    });

    ui.separator();

    // ── Chat Log ──────────────────────────────────────────────────────

    ui.collapsing("Chat Log (last 10)", |ui| {
        if chat.is_empty() {
            ui.label("No chat messages received yet.");
        } else {
            egui::ScrollArea::vertical()
                .max_height(200.0)
                .show(ui, |ui| {
                    for (sender, message) in &chat {
                        ui.monospace(format!("<{sender}> {message}"));
                    }
                });
        }
    });
}
