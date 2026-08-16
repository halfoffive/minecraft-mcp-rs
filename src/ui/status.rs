//! Status panel: connection state, player info, nearby stats, chat log.
//!
//! Reads lock-free from [`SharedState`] via [`ArcSwap::load`](arc_swap::ArcSwap::load)
//! for the world snapshot, and uses short-lived read locks for config and stats.
//!
//! All user-visible strings are translated via the [`i18n`] layer.

use egui::Ui;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::i18n::{self, TextKey};
use crate::state::{McpServerStatus, SharedState};

/// Compute the command success rate as a percentage.
///
/// Returns `None` when there is nothing to measure yet (`processed == 0`),
/// so the UI can omit the rate line instead of showing "0%". The rate is
/// clamped to `[0, 100]` — `succeeded` should never exceed `processed`, but
/// the counters are updated from a racing executor, so a defensive clamp
/// keeps the display honest.
pub fn success_rate(processed: u64, succeeded: u64) -> Option<f64> {
    if processed == 0 {
        return None;
    }
    let rate = (succeeded as f64 / processed as f64) * 100.0;
    Some(rate.clamp(0.0, 100.0))
}

/// Cache of the last chat messages rendered by the status panel (L-20).
///
/// [`SharedState::get_chat_messages`] clones every `(sender, message)` pair
/// (up to the 50-message cap) — paying for that on every frame was wasteful
/// when nothing new arrived. The cache stores the chat cursor it was built
/// from; when the cursor advances, the messages are re-fetched exactly once.
#[derive(Debug, Clone, Default)]
pub struct ChatCache {
    /// `SharedState::chat_cursor()` of the last fetch (`None` = never
    /// fetched — the first `get` always fetches, even for an empty history
    /// whose cursor is 0).
    cursor: Option<u64>,
    /// The cloned messages shown by the panel.
    messages: Vec<(String, String)>,
    /// Number of fetches performed (test diagnostic proving reuse).
    fetches: u64,
}

impl ChatCache {
    /// Create an empty chat cache (first `get` always fetches).
    pub fn new() -> Self {
        Self::default()
    }

    /// Return the chat messages to render, re-fetching from `state` only
    /// when the chat cursor advanced past the cached cursor.
    pub fn get(&mut self, state: &Arc<SharedState>) -> &[(String, String)] {
        let cursor = state.chat_cursor();
        if self.cursor != Some(cursor) {
            self.cursor = Some(cursor);
            self.messages = state.get_chat_messages();
            self.fetches += 1;
        }
        &self.messages
    }

    /// Number of fetches performed (0 before the first `get`).
    pub fn fetches(&self) -> u64 {
        self.fetches
    }
}

/// Render the status panel.
///
/// Displays:
/// - Last error message (red banner, only when present)
/// - Connection status (online/offline with uptime)
/// - MCP server status (running address / stdio / failed / stopped)
/// - Player information (position, health, hunger, gamemode)
/// - World stats (blocks, entities, chunks loaded)
/// - Command counters (processed, succeeded, failed)
/// - Last 50 chat messages
pub fn status_panel(ui: &mut Ui, state: &Arc<SharedState>, chat_cache: &mut ChatCache) {
    let (is_online, is_connecting) = (state.is_online(), state.is_connecting());
    let snapshot = state.read_snapshot();
    // Read connected_since under the lock, then drop the guard immediately.
    // The atomic counters (commands_processed etc.) don't need the lock.
    let connected_since = state.read_run_stats().connected_since;
    // L-20: reuse the cached messages while the chat cursor is unchanged
    // instead of cloning all 50 pairs every frame.
    let chat = chat_cache.get(state);

    // ── Last Error ────────────────────────────────────────────────────
    // Display a prominent red banner if the bot/MCP layer has reported an
    // error.  When there is no error, nothing is rendered (no empty row).
    // The "×" button next to the message lets the user dismiss the banner
    // without restarting the app — useful when the error has been
    // acknowledged and the user wants to see the rest of the status panel
    // without the banner taking up space.
    if let Some(msg) = state.last_error() {
        ui.horizontal(|ui| {
            ui.colored_label(
                egui::Color32::RED,
                format!("{} {}", i18n::tr(TextKey::Error), msg),
            );
            // Dismiss button — a small "×" styled as a button. Clicking
            // clears `last_error` so the banner disappears on the next
            // frame. We use a Button with no background to keep the row
            // compact.
            let dismiss = egui::Button::new("×")
                .small()
                .fill(egui::Color32::TRANSPARENT)
                .stroke(egui::Stroke::NONE);
            if ui.add(dismiss).clicked() {
                state.clear_last_error();
            }
        });
        ui.separator();
    }

    // ── Connection ────────────────────────────────────────────────────

    ui.horizontal(|ui| {
        ui.label(i18n::tr(TextKey::Connection));
        if is_online {
            ui.colored_label(egui::Color32::GREEN, i18n::tr(TextKey::Online));
        } else if is_connecting {
            // Connecting state: show a spinner next to the "Connecting…"
            // label so the user can see the app is actively trying to
            // connect (rather than just stuck on a static label).
            ui.add(egui::Spinner::new());
            ui.colored_label(egui::Color32::YELLOW, i18n::tr(TextKey::Connecting));
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

    // ── MCP Server ────────────────────────────────────────────────────
    // Surface MCP server state (running address, stdio, bind failure, or
    // stopped) so the user can see at a glance whether the MCP server is
    // accepting requests — in particular, port-in-use bind failures that
    // would otherwise only appear in logs.
    let mcp_status = state.get_mcp_server_status();
    match &mcp_status {
        McpServerStatus::Running(addr) => {
            ui.label(format!(
                "{} {}",
                i18n::tr(TextKey::McpServerLabel),
                i18n::tr(TextKey::McpServerRunning).replace("{addr}", &addr.to_string())
            ));
        }
        McpServerStatus::Stdio => {
            ui.label(format!(
                "{} {}",
                i18n::tr(TextKey::McpServerLabel),
                i18n::tr(TextKey::McpServerStdio)
            ));
        }
        McpServerStatus::Failed(msg) => {
            ui.label(
                egui::RichText::new(format!(
                    "{} {}",
                    i18n::tr(TextKey::McpServerLabel),
                    i18n::tr(TextKey::McpServerFailed).replace("{msg}", msg)
                ))
                .color(egui::Color32::from_rgb(220, 80, 80)),
            );
        }
        McpServerStatus::Stopped => {
            ui.label(format!(
                "{} {}",
                i18n::tr(TextKey::McpServerLabel),
                i18n::tr(TextKey::McpServerStopped)
            ));
        }
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

        if let Some(rate) = success_rate(processed, succeeded) {
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
                    for (sender, message) in chat {
                        // Chat line format "<sender> message" is universal
                        // across languages; left untranslated on purpose.
                        ui.monospace(format!("<{sender}> {message}"));
                    }
                });
        }
    });
}

// ═══════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::state::SharedState;
    use std::sync::Arc;

    /// No processed commands → no rate (UI omits the line).
    #[test]
    fn test_success_rate_none_when_nothing_processed() {
        assert_eq!(success_rate(0, 0), None);
        assert_eq!(success_rate(0, 5), None);
    }

    #[test]
    fn test_success_rate_full() {
        let rate = success_rate(10, 10).expect("rate exists");
        assert!((rate - 100.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_success_rate_partial() {
        let rate = success_rate(4, 3).expect("rate exists");
        assert!((rate - 75.0).abs() < 1e-9);
    }

    /// A racing executor can momentarily make `succeeded > processed`; the
    /// rate must be clamped rather than reporting >100%.
    #[test]
    fn test_success_rate_clamps_over_100() {
        let rate = success_rate(2, 5).expect("rate exists");
        assert!(rate <= 100.0);
    }

    // -- Chat cache (L-20) ----------------------------------------------------

    /// L-20: the status panel cloned all 50 chat messages every frame. The
    /// cache reuses the cloned messages while the chat cursor is unchanged
    /// and refetches exactly once when it advances.
    #[test]
    fn test_status_chat_cache_reuses_on_same_cursor() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        state.add_chat_message("Alice".into(), "hello".into());

        let mut cache = ChatCache::new();
        {
            let first = cache.get(&state);
            assert_eq!(first.len(), 1);
            assert_eq!(first[0], ("Alice".into(), "hello".into()));
        }
        assert_eq!(cache.fetches(), 1);

        // Same cursor → no refetch (this is the per-frame hot path).
        {
            let second = cache.get(&state);
            assert_eq!(second.len(), 1);
        }
        assert_eq!(cache.fetches(), 1, "same cursor must reuse the cache");

        // New message advances the cursor → exactly one refetch.
        state.add_chat_message("Bob".into(), "world".into());
        {
            let third = cache.get(&state);
            assert_eq!(third.len(), 2);
        }
        assert_eq!(cache.fetches(), 2);
    }

    /// An empty chat history is handled: one initial fetch, then cache hits.
    #[test]
    fn test_status_chat_cache_empty_history() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let mut cache = ChatCache::new();
        assert!(cache.get(&state).is_empty());
        assert!(cache.get(&state).is_empty());
        assert_eq!(cache.fetches(), 1);
    }
}
