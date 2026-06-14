//! Settings panel: editable [`AppConfig`] fields + Connect / Disconnect buttons.
//!
//! All config fields are rendered as editable widgets (text inputs for strings,
//! [`DragValue`] for numbers).  Edits accumulate in [`EditConfig`] which is
//! persisted to [`SharedState`] when the user clicks **Connect**.  The
//! **Disconnect** button sets the bot offline.

use egui::{DragValue, TextEdit, Ui};
use std::sync::Arc;

use crate::channel::BotCommandSender;
use crate::state::SharedState;
use crate::ui::app::EditConfig;

/// Render the settings panel.
///
/// Returns `true` when the Connect button is clicked (caller should persist
/// edits and spawn the connection task).
pub fn settings_panel(
    ui: &mut Ui,
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    edit: &mut EditConfig,
) -> bool {
    let mut connect_clicked = false;

    // ── Minecraft Server ──────────────────────────────────────
    ui.label("Minecraft Server");
    ui.horizontal(|ui| {
        ui.label("Address:");
        ui.add(TextEdit::singleline(&mut edit.mc_address).desired_width(180.0));
    });
    ui.horizontal(|ui| {
        ui.label("Port:");
        ui.add(DragValue::new(&mut edit.mc_port).range(1..=65535));
    });

    ui.separator();

    // ── Bot Identity ──────────────────────────────────────────
    ui.label("Bot Identity");
    ui.horizontal(|ui| {
        ui.label("Username:");
        ui.add(TextEdit::singleline(&mut edit.ai_username).desired_width(180.0));
    });

    ui.separator();

    // ── MCP Server ────────────────────────────────────────────
    ui.label("MCP Server");
    ui.horizontal(|ui| {
        ui.label("Bind Address:");
        ui.add(TextEdit::singleline(&mut edit.mcp_address).desired_width(180.0));
    });
    ui.horizontal(|ui| {
        ui.label("Bind Port:");
        ui.add(DragValue::new(&mut edit.mcp_port).range(1..=65535));
    });

    ui.separator();

    // ── Scanning ──────────────────────────────────────────────
    ui.label("Scanning");
    ui.horizontal(|ui| {
        ui.label("Chunk Scan Radius (1-16):");
        ui.add(DragValue::new(&mut edit.chunk_scan_radius).range(1..=16));
    });
    ui.horizontal(|ui| {
        ui.label("Block Perception Radius (8-64):");
        ui.add(DragValue::new(&mut edit.block_perception_radius).range(8..=64));
    });
    ui.horizontal(|ui| {
        ui.label("Snapshot Interval (ms):");
        ui.add(DragValue::new(&mut edit.snapshot_interval_ms).range(100..=10000));
    });

    ui.separator();

    // ── Timing ────────────────────────────────────────────────
    ui.label("Timing");
    ui.horizontal(|ui| {
        ui.label("Reconnect Initial Delay (ms):");
        ui.add(DragValue::new(&mut edit.reconnect_initial_delay_ms).range(1000..=300000));
    });
    ui.horizontal(|ui| {
        ui.label("Reconnect Max Delay (ms):");
        ui.add(DragValue::new(&mut edit.reconnect_max_delay_ms).range(5000..=600000));
    });
    ui.horizontal(|ui| {
        ui.label("Command Timeout (s):");
        ui.add(DragValue::new(&mut edit.command_timeout_secs).range(1..=300));
    });

    ui.separator();

    // ── Connect / Disconnect ──────────────────────────────────
    let is_online = state.is_online();

    ui.horizontal(|ui| {
        let connect_btn = ui.add_enabled(!is_online, egui::Button::new("🔌 Connect"));
        let disconnect_btn = ui.add_enabled(is_online, egui::Button::new("⏻ Disconnect"));

        if connect_btn.clicked() {
            tracing::info!("Connect button pressed");
            let _ = sender;
            connect_clicked = true;
        }

        if disconnect_btn.clicked() {
            tracing::info!("Disconnect button pressed — setting bot offline");
            state.set_online(false);
            let _ = sender;
        }
    });

    if is_online {
        ui.colored_label(egui::Color32::GREEN, "● Connected");
    } else {
        ui.colored_label(egui::Color32::RED, "● Disconnected");
    }

    connect_clicked
}
