//! Settings panel: AppConfig fields + Connect/Disconnect buttons.
//!
//! Renders each [`AppConfig`] field as a read-only label (full editing with
//! text inputs requires mutable local state, planned for a follow-up).
//! Connect and Disconnect buttons dispatch bot lifecycle commands through
//! the [`BotCommandSender`] channel.

use egui::Ui;
use std::sync::Arc;

use crate::channel::BotCommandSender;
use crate::state::SharedState;

/// Render the settings panel.
///
/// Displays all [`AppConfig`] fields and provides Connect / Disconnect
/// buttons that toggle the bot connection.  Fields are currently read-only;
/// the config can be updated programmatically via the MCP server.
pub fn settings_panel(ui: &mut Ui, state: &Arc<SharedState>, sender: &BotCommandSender) {
    let config = state.read_config().clone();

    ui.label("Minecraft Server");
    ui.horizontal(|ui| {
        ui.label("Address:");
        ui.label(&config.mc_address);
    });
    ui.horizontal(|ui| {
        ui.label("Port:");
        ui.label(config.mc_port.to_string());
    });

    ui.separator();
    ui.label("Bot Identity");
    ui.horizontal(|ui| {
        ui.label("Username:");
        ui.label(&config.ai_username);
    });

    ui.separator();
    ui.label("MCP Server");
    ui.horizontal(|ui| {
        ui.label("Bind Address:");
        ui.label(&config.mcp_address);
    });
    ui.horizontal(|ui| {
        ui.label("Bind Port:");
        ui.label(config.mcp_port.to_string());
    });

    ui.separator();
    ui.label("Scanning");
    ui.horizontal(|ui| {
        ui.label("Chunk Scan Radius (1-16):");
        ui.label(config.chunk_scan_radius.to_string());
    });
    ui.horizontal(|ui| {
        ui.label("Block Perception Radius (8-64):");
        ui.label(config.block_perception_radius.to_string());
    });
    ui.horizontal(|ui| {
        ui.label("Snapshot Interval (ms):");
        ui.label(config.snapshot_interval_ms.to_string());
    });

    ui.separator();
    ui.label("Timing");
    ui.horizontal(|ui| {
        ui.label("Reconnect Initial Delay (ms):");
        ui.label(config.reconnect_initial_delay_ms.to_string());
    });
    ui.horizontal(|ui| {
        ui.label("Reconnect Max Delay (ms):");
        ui.label(config.reconnect_max_delay_ms.to_string());
    });
    ui.horizontal(|ui| {
        ui.label("Command Timeout (s):");
        ui.label(config.command_timeout_secs.to_string());
    });

    ui.separator();

    let is_online = state.is_online();

    ui.horizontal(|ui| {
        let connect_btn = ui.button("🔌 Connect");
        let disconnect_btn = ui.button("⏻ Disconnect");

        if connect_btn.clicked() && !is_online {
            tracing::info!("Connect button pressed — dispatching connect command");
            let _ = sender;
            // TODO: dispatch BotCommand to initiate connection
        }

        if disconnect_btn.clicked() && is_online {
            tracing::info!("Disconnect button pressed — dispatching disconnect command");
            let _ = sender;
            // TODO: dispatch BotCommand to terminate connection
        }
    });

    if is_online {
        ui.colored_label(egui::Color32::GREEN, "● Connected");
    } else {
        ui.colored_label(egui::Color32::RED, "● Disconnected");
    }
}
