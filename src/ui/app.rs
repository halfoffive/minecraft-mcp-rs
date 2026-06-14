//! Application shell: window setup, layout, dispatch.
//!
//! [`MinecraftApp`] implements [`eframe::App`] and renders the settings
//! and status panels inside a central layout.  It requests periodic repaints
//! at ~10 FPS so that live state changes (bot connection, world snapshot,
//! chat messages) are reflected in the UI without a manual refresh.
//!
//! # Threading
//!
//! The egui render loop runs on the **main thread**.  The MCP server runs on
//! a background OS thread with its own tokio runtime.  The optional bot
//! connection is also spawned on a dedicated OS thread because azalea's
//! [`ClientBuilder::start`] internally creates a `LocalSet` which is `!Send`.

use std::sync::Arc;

use eframe::App;
use egui::Context;

use crate::bot::connection::ConnectionManager;
use crate::channel::{BotCommandReceiver, BotCommandSender};
use crate::config::AppConfig;
use crate::state::SharedState;
use crate::ui::{settings, status};

/// Main egui application shell.
pub struct MinecraftApp {
    /// Shared state accessed lock-free for the world snapshot,
    /// and with short-lived read locks for config and stats.
    state: Arc<SharedState>,
    /// Channel sender for dispatching bot commands.
    sender: BotCommandSender,
    /// Shared command receiver, passed to the bot connection task so it can
    /// process commands from the MCP server while connected.
    command_receiver: Arc<tokio::sync::Mutex<BotCommandReceiver>>,
    /// Local edit buffers for the settings panel.  Initialised from
    /// [`SharedState`] config on first frame.
    edit_config: Option<EditConfig>,
}

/// Mutable copy of every [`AppConfig`] field for the settings panel.
///
/// We keep a local clone so that egui text edits don't require holding
/// the config write lock across frames.
#[derive(Clone, Debug)]
pub struct EditConfig {
    pub mc_address: String,
    pub mc_port: u16,
    pub ai_username: String,
    pub mcp_address: String,
    pub mcp_port: u16,
    pub task_name: String,
    pub chunk_scan_radius: u8,
    pub block_perception_radius: u8,
    pub snapshot_interval_ms: u64,
    pub reconnect_initial_delay_ms: u64,
    pub reconnect_max_delay_ms: u64,
    pub command_timeout_secs: u64,
}

impl From<&AppConfig> for EditConfig {
    fn from(cfg: &AppConfig) -> Self {
        Self {
            mc_address: cfg.mc_address.clone(),
            mc_port: cfg.mc_port,
            ai_username: cfg.ai_username.clone(),
            mcp_address: cfg.mcp_address.clone(),
            mcp_port: cfg.mcp_port,
            task_name: cfg.task_name.clone(),
            chunk_scan_radius: cfg.chunk_scan_radius,
            block_perception_radius: cfg.block_perception_radius,
            snapshot_interval_ms: cfg.snapshot_interval_ms,
            reconnect_initial_delay_ms: cfg.reconnect_initial_delay_ms,
            reconnect_max_delay_ms: cfg.reconnect_max_delay_ms,
            command_timeout_secs: cfg.command_timeout_secs,
        }
    }
}

impl EditConfig {
    /// Write the edited values back into [`SharedState`] config.
    pub(crate) fn apply(&self, state: &SharedState) {
        state.update_config(|cfg| {
            cfg.mc_address = self.mc_address.clone();
            cfg.mc_port = self.mc_port;
            cfg.ai_username = self.ai_username.clone();
            cfg.mcp_address = self.mcp_address.clone();
            cfg.mcp_port = self.mcp_port;
            cfg.task_name = self.task_name.clone();
            cfg.chunk_scan_radius = self.chunk_scan_radius;
            cfg.block_perception_radius = self.block_perception_radius;
            cfg.snapshot_interval_ms = self.snapshot_interval_ms;
            cfg.reconnect_initial_delay_ms = self.reconnect_initial_delay_ms;
            cfg.reconnect_max_delay_ms = self.reconnect_max_delay_ms;
            cfg.command_timeout_secs = self.command_timeout_secs;
        });
    }
}

impl MinecraftApp {
    /// Create a new [`MinecraftApp`].
    pub fn new(
        state: Arc<SharedState>,
        sender: BotCommandSender,
        command_receiver: Arc<tokio::sync::Mutex<BotCommandReceiver>>,
    ) -> Self {
        Self {
            state,
            sender,
            command_receiver,
            edit_config: None,
        }
    }

    /// Start the bot connection on a dedicated OS thread.
    ///
    /// We spawn a new thread (rather than using `tokio::spawn`) because
    /// azalea's `ClientBuilder::start` internally creates a `LocalSet`
    /// which is `!Send`.
    fn connect_bot(&self) {
        let config = self.state.read_config().clone();
        let state = Arc::clone(&self.state);
        let receiver = Arc::clone(&self.command_receiver);

        std::thread::Builder::new()
            .name("bot-connection".into())
            .spawn(move || {
                let rt = tokio::runtime::Runtime::new()
                    .expect("Failed to create tokio runtime for bot connection");
                let manager = ConnectionManager::new(config, Arc::clone(&state));

                rt.block_on(async move {
                    if let Err(e) = manager.connect(receiver, None).await {
                        tracing::error!(error = %e, "bot connection task failed");
                    }
                });

                tracing::info!("Bot connection thread exited");
            })
            .expect("Failed to spawn bot connection thread");

        tracing::info!("Bot connection thread spawned");
    }
}

impl App for MinecraftApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        // Request continuous repaint at ~10 FPS so that live status
        // updates (connection state, chat, command counters) are visible
        // without user interaction.
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(0.1));

        // Lazy-init the edit buffers from current config.
        if self.edit_config.is_none() {
            let cfg = self.state.read_config();
            self.edit_config = Some(EditConfig::from(&*cfg));
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Minecraft MCP Server");
            ui.separator();

            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.collapsing("⚙ Settings", |ui| {
                    if let Some(ref mut edit) = self.edit_config {
                        let connect_clicked =
                            settings::settings_panel(ui, &self.state, &self.sender, edit);

                        if connect_clicked {
                            // Persist edits before connecting.
                            edit.apply(&self.state);
                            self.connect_bot();
                        }
                    }
                });

                ui.collapsing("📊 Status", |ui| {
                    status::status_panel(ui, &self.state);
                });
            });
        });
    }
}
