//! Application shell: window setup, layout, dispatch.
//!
//! [`MinecraftApp`] implements [`eframe::App`] and renders the settings
//! and status panels inside a central layout.  It requests periodic repaints
//! at ~10 FPS so that live state changes (bot connection, world snapshot,
//! chat messages) are reflected in the UI without a manual refresh.
//!
//! # Threading
//!
//! The egui render loop runs on the **main thread**.  The MCP server runs
//! in a background thread with its own tokio runtime.  All shared state is
//! accessed lock-free via [`arc_swap::ArcSwap`] (world snapshot) and
//! [`std::sync::atomic`] primitives (online flag, command counters).

use std::sync::Arc;

use eframe::App;
use egui::Context;

use crate::channel::BotCommandSender;
use crate::state::SharedState;
use crate::ui::{settings, status};

/// Main egui application shell.
///
/// Holds:
/// - [`Arc<SharedState>`] — read lock-free by the UI every frame.
/// - [`BotCommandSender`] — passed to the settings panel for Connect/Disconnect.
pub struct MinecraftApp {
    /// Shared state accessed lock-free for the world snapshot,
    /// and with short-lived read locks for config and stats.
    state: Arc<SharedState>,
    /// Channel sender for dispatching bot lifecycle commands
    /// (Connect, Disconnect) from the settings panel to the bot engine.
    sender: BotCommandSender,
}

impl MinecraftApp {
    /// Create a new [`MinecraftApp`].
    ///
    /// The `state` and `sender` are shared with the MCP server running in
    /// a background thread.  The UI reads `state` lock-free every frame.
    pub fn new(state: Arc<SharedState>, sender: BotCommandSender) -> Self {
        Self { state, sender }
    }
}

impl App for MinecraftApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        // Request continuous repaint at ~10 FPS so that live status
        // updates (connection state, chat, command counters) are visible
        // without user interaction.
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(0.1));

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Minecraft MCP Server");
            ui.separator();

            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.collapsing("⚙ Settings", |ui| {
                    settings::settings_panel(ui, &self.state, &self.sender);
                });

                ui.collapsing("📊 Status", |ui| {
                    status::status_panel(ui, &self.state);
                });
            });
        });
    }
}
