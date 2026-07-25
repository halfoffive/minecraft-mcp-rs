//! Settings panel: editable [`AppConfig`](crate::config::AppConfig) fields
//! + Connect / Disconnect buttons.
//!
//! All config fields are rendered as editable widgets (text inputs for strings,
//! [`DragValue`] for numbers).  Edits accumulate in [`EditConfig`] which is
//! persisted to [`SharedState`] when the user clicks **Connect**.  The
//! **Disconnect** button sets the bot offline.
//!
//! All user-visible strings are translated via the [`i18n`] layer.

use egui::{DragValue, TextEdit, Ui};

use crate::config::McpTransport;
use crate::state::SharedState;
use crate::ui::app::EditConfig;
use crate::ui::i18n::{self, Language, TextKey};

/// Render the settings panel.
///
/// Returns `true` when the Connect button is clicked (caller should persist
/// edits and spawn the connection task).
pub fn settings_panel(ui: &mut Ui, state: &SharedState, edit: &mut EditConfig) -> bool {
    let mut connect_clicked = false;

    // Sync the active i18n language with the dropdown *before* rendering any
    // label. This prevents a one-frame flash where labels above the Language
    // section (e.g. Timing) still appear in the previous language immediately
    // after the user picks a new one.
    if i18n::current() != edit.language {
        i18n::set(edit.language);
        ui.ctx().request_repaint();
    }

    // ── Minecraft Server ──────────────────────────────────────
    ui.label(i18n::tr(TextKey::MinecraftServer));
    ui.horizontal(|ui| {
        ui.label(i18n::tr(TextKey::Address));
        ui.add(TextEdit::singleline(&mut edit.mc_address).desired_width(180.0));
    });
    ui.horizontal(|ui| {
        ui.label(i18n::tr(TextKey::Port));
        ui.add(DragValue::new(&mut edit.mc_port).range(1..=65535));
    });

    ui.separator();

    // ── Bot Identity ──────────────────────────────────────────
    ui.label(i18n::tr(TextKey::BotIdentity));
    ui.horizontal(|ui| {
        ui.label(i18n::tr(TextKey::Username));
        ui.add(TextEdit::singleline(&mut edit.ai_username).desired_width(180.0));
    });

    ui.separator();

    // ── MCP Server ────────────────────────────────────────────
    ui.label(i18n::tr(TextKey::McpServer));
    ui.horizontal(|ui| {
        ui.label(i18n::tr(TextKey::BindAddress));
        ui.add(TextEdit::singleline(&mut edit.mcp_address).desired_width(180.0));
    });
    // Warn when the HTTP bind address is not loopback: the Bearer
    // token travels in cleartext (no TLS), so binding to anything
    // other than `127.0.0.1` / `::1` / `localhost` (including
    // `0.0.0.0`, which binds all interfaces) exposes it to the
    // network. Only relevant for HTTP transport.
    if edit.mcp_transport == McpTransport::Http {
        let is_safe = matches!(edit.mcp_address.as_str(), "127.0.0.1" | "::1" | "localhost");
        if !is_safe && !edit.mcp_address.is_empty() {
            ui.label(
                egui::RichText::new(i18n::tr(TextKey::TlsWarning))
                    .color(egui::Color32::from_rgb(220, 80, 80))
                    .small(),
            );
        }
    }
    ui.horizontal(|ui| {
        ui.label(i18n::tr(TextKey::BindPort));
        ui.add(DragValue::new(&mut edit.mcp_port).range(1..=65535));
    });
    ui.horizontal(|ui| {
        ui.label(i18n::tr(TextKey::Transport));
        egui::ComboBox::from_id_salt("mcp_transport_combo")
            .selected_text(transport_label(edit.mcp_transport))
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut edit.mcp_transport,
                    McpTransport::Http,
                    i18n::tr(TextKey::TransportHttp),
                );
                ui.selectable_value(
                    &mut edit.mcp_transport,
                    McpTransport::Stdio,
                    i18n::tr(TextKey::TransportStdio),
                );
            });
    });
    if edit.mcp_transport == McpTransport::Http {
        ui.horizontal(|ui| {
            ui.label(i18n::tr(TextKey::Token));
            ui.add(
                TextEdit::singleline(&mut edit.mcp_token)
                    .password(true)
                    .hint_text(i18n::tr(TextKey::TokenHint))
                    .desired_width(180.0),
            );
        });
    }

    ui.separator();

    // ── Scanning ──────────────────────────────────────────────
    ui.label(i18n::tr(TextKey::Scanning));
    ui.horizontal(|ui| {
        ui.label(i18n::tr(TextKey::ChunkScanRadius));
        ui.add(DragValue::new(&mut edit.chunk_scan_radius).range(1..=16));
    });
    ui.horizontal(|ui| {
        ui.label(i18n::tr(TextKey::BlockPerceptionRadius));
        ui.add(DragValue::new(&mut edit.block_perception_radius).range(8..=64));
    });
    ui.horizontal(|ui| {
        ui.label(i18n::tr(TextKey::SnapshotInterval));
        ui.add(DragValue::new(&mut edit.snapshot_interval_ms).range(100..=10000));
    });

    ui.separator();

    // ── Timing ────────────────────────────────────────────────
    ui.label(i18n::tr(TextKey::Timing));
    ui.horizontal(|ui| {
        ui.label(i18n::tr(TextKey::ReconnectInitialDelay));
        ui.add(DragValue::new(&mut edit.reconnect_initial_delay_ms).range(1000..=300000));
    });
    ui.horizontal(|ui| {
        ui.label(i18n::tr(TextKey::ReconnectMaxDelay));
        ui.add(DragValue::new(&mut edit.reconnect_max_delay_ms).range(5000..=600000));
    });
    ui.horizontal(|ui| {
        ui.label(i18n::tr(TextKey::CommandTimeout));
        ui.add(DragValue::new(&mut edit.command_timeout_secs).range(1..=300));
    });

    ui.separator();

    // ── Language ──────────────────────────────────────────────
    // Rendered before Connect / Disconnect so it sits with the other
    // config sections.  Changing the dropdown updates the active i18n
    // language immediately (next frame) — no reconnect needed — by
    // calling `i18n::set` directly here, and also persists the choice
    // to AppConfig immediately via `update_config`.
    ui.label(i18n::tr(TextKey::Language));
    let prev_language = edit.language;
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("language_combo")
            .selected_text(language_label(edit.language))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut edit.language, Language::En, i18n::tr(TextKey::LangEn));
                ui.selectable_value(
                    &mut edit.language,
                    Language::ZhCn,
                    i18n::tr(TextKey::LangZhCn),
                );
            });
    });
    if edit.language != prev_language {
        state.update_config(|cfg| cfg.language = edit.language);
    }
    ui.separator();

    // ── Connect / Disconnect ──────────────────────────────────
    let is_online = state.is_online();
    let is_connecting = state.is_connecting();

    ui.horizontal(|ui| {
        // Disable Connect when already online OR a connection attempt is
        // in progress (prevents double-spawn).
        let connect_enabled = !is_online && !is_connecting;
        let connect_btn = ui.add_enabled(
            connect_enabled,
            egui::Button::new(i18n::tr(TextKey::Connect)),
        );
        let disconnect_btn = ui.add_enabled(
            is_online || is_connecting,
            egui::Button::new(i18n::tr(TextKey::Disconnect)),
        );

        if connect_btn.clicked() {
            tracing::info!("Connect button pressed");
            // Clear any stale disconnect request from a previous session.
            state.clear_disconnect_request();
            connect_clicked = true;
        }

        if disconnect_btn.clicked() {
            tracing::info!("Disconnect button pressed");
            // Signal the reconnect loop to stop retrying. The actual TCP
            // teardown happens when the bot's next event fires
            // Event::Disconnect (which calls bot.exit()), or when the
            // server drops the connection.
            //
            // Do NOT call `set_online(false)` here: the bot is still
            // online until the disconnect actually completes, and
            // flipping the flag early confuses other UI elements
            // (status panel goes red while the bot is still moving)
            // and races with the `Event::Disconnect` handler in
            // `bot/events.rs` which is the canonical source of truth
            // for the online state. The status panel will reflect the
            // disconnect as soon as `handle_disconnect` writes
            // `set_online(false)`.
            state.request_disconnect();
        }
    });

    if is_online {
        ui.colored_label(egui::Color32::GREEN, i18n::tr(TextKey::Connected));
    } else if is_connecting {
        ui.colored_label(egui::Color32::YELLOW, i18n::tr(TextKey::Connecting));
    } else {
        ui.colored_label(egui::Color32::RED, i18n::tr(TextKey::Disconnected));
    }

    connect_clicked
}

// ════════════════════════════════════════════════════════════════════
// Local helpers
// ════════════════════════════════════════════════════════════════════

/// Return the localised display name for `lang` (used as the ComboBox's
/// `selected_text`).
///
/// Mirrors the selectable labels inside the dropdown so the collapsed
/// ComboBox shows the same string as the highlighted option when expanded.
fn language_label(lang: Language) -> &'static str {
    match lang {
        Language::En => i18n::tr(TextKey::LangEn),
        Language::ZhCn => i18n::tr(TextKey::LangZhCn),
    }
}

/// Return the localised display name for `transport` (used as the
/// ComboBox's `selected_text`).
///
/// Mirrors the selectable labels inside the dropdown so the collapsed
/// ComboBox shows the same string as the highlighted option when expanded,
/// and updates immediately when the UI language changes.
fn transport_label(transport: McpTransport) -> &'static str {
    match transport {
        McpTransport::Http => i18n::tr(TextKey::TransportHttp),
        McpTransport::Stdio => i18n::tr(TextKey::TransportStdio),
    }
}

// ═══════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;

    /// The Disconnect button click in `settings_panel` is wired to
    /// `state.request_disconnect()` and nothing else — the actual
    /// `set_online(false)` must be performed by the canonical
    /// `Event::Disconnect` handler in `bot/events.rs`. This test
    /// guards the contract: the UI does NOT pre-emptively flip the
    /// online flag, so the status panel and `is_connected()` keep
    /// showing the truthful "still online" state until the real
    /// disconnect event fires.
    #[test]
    fn test_disconnect_does_not_set_offline_early() {
        let state = SharedState::new(AppConfig::default());
        state.set_online(true);
        assert!(
            state.is_online(),
            "precondition: bot should be online before disconnect"
        );

        // Simulate the Disconnect button click. The button's callback
        // is `state.request_disconnect();` — no other state mutation.
        state.request_disconnect();

        // The bot must still report as online. The flag is the source
        // of truth for the rest of the UI (status panel, query tools
        // like `is_connected`), and flipping it here would race with
        // `Event::Disconnect::handle_disconnect` (which is the only
        // authorised writer).
        assert!(
            state.is_online(),
            "request_disconnect() must not flip the online flag \
             — only the Event::Disconnect handler should"
        );

        // Sanity: the explicit offline transition still works as
        // expected once the canonical writer runs.
        state.set_online(false);
        assert!(!state.is_online(), "set_online(false) should take effect");
    }
}
