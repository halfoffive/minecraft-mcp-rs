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
//! [`ClientBuilder::start`](azalea::ClientBuilder::start) internally creates a
//! `LocalSet` which is `!Send`.

use std::sync::Arc;
use std::thread::JoinHandle;

use eframe::App;
use egui::Context;

use crate::bot::connection::ConnectionManager;
use crate::channel::{BotCommandReceiver, BotCommandSender};
use crate::config::{AppConfig, McpTransport};
use crate::state::SharedState;
use crate::ui::i18n::Language;
use crate::ui::{mcp_config, settings, status};

/// Main egui application shell.
pub struct MinecraftApp {
    /// Shared state accessed lock-free for the world snapshot,
    /// and with short-lived read locks for config and stats.
    state: Arc<SharedState>,
    /// Shared command receiver slot, passed to the bot connection task so it
    /// can process commands from the MCP server while connected. The receiver
    /// is leased out to the command executor on `Event::Spawn`.
    command_receiver: Arc<std::sync::Mutex<Option<BotCommandReceiver>>>,
    /// Handle to the bot-connection OS thread (if running). Joined on Drop
    /// so the process exits cleanly when the window closes.
    bot_thread: Option<JoinHandle<()>>,
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
    /// Bearer token presented by MCP clients over HTTP auth.
    pub mcp_token: String,
    /// Transport mechanism the MCP server uses to talk to clients.
    pub mcp_transport: McpTransport,
    /// UI display language (mirrors [`AppConfig::language`]).
    pub language: Language,
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
            mcp_token: cfg.mcp_token.clone(),
            mcp_transport: cfg.mcp_transport,
            language: cfg.language,
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
            cfg.mcp_token = self.mcp_token.clone();
            cfg.mcp_transport = self.mcp_transport;
            cfg.language = self.language;
        });
    }
}

impl MinecraftApp {
    /// Create a new [`MinecraftApp`].
    pub fn new(
        state: Arc<SharedState>,
        _sender: BotCommandSender,
        command_receiver: Arc<std::sync::Mutex<Option<BotCommandReceiver>>>,
    ) -> Self {
        // _sender is intentionally unused here — the MCP server thread holds
        // its own clone and is the sole consumer of the command channel. The
        // parameter is retained to keep the main.rs wiring simple and allow
        // future UI-driven commands without a signature change.
        Self {
            state,
            command_receiver,
            bot_thread: None,
            edit_config: None,
        }
    }

    /// Start the bot connection on a dedicated OS thread.
    ///
    /// We spawn a new thread (rather than using `tokio::spawn`) because
    /// azalea's `ClientBuilder::start` internally creates a `LocalSet`
    /// which is `!Send`.
    ///
    /// Uses [`SharedState::try_begin_connecting`] to guard against
    /// double-spawn if the user clicks Connect while a previous attempt
    /// is still in progress.
    fn connect_bot(&mut self) {
        if !self.state.try_begin_connecting() {
            tracing::warn!("Connect clicked but a connection attempt is already in progress");
            return;
        }

        let config = self.state.read_config().clone();
        let state = Arc::clone(&self.state);
        let receiver = Arc::clone(&self.command_receiver);

        let handle = std::thread::Builder::new()
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

                // Clear the connecting flag in case the loop exited without
                // doing it (e.g. due to an early error return).
                state.clear_connecting();
                tracing::info!("Bot connection thread exited");
            })
            .expect("Failed to spawn bot connection thread");

        self.bot_thread = Some(handle);
        tracing::info!("Bot connection thread spawned");
    }
}

impl Drop for MinecraftApp {
    fn drop(&mut self) {
        // Signal the bot to stop retrying and let the connection thread
        // exit cleanly when the window is closed.
        self.state.request_disconnect();
        self.state.set_online(false);

        if let Some(handle) = self.bot_thread.take() {
            // Try to join with a 3-second timeout to avoid hanging the UI
            // thread. The bot thread runs its own tokio runtime; with
            // `disconnect_requested` set and the cancel token tripped, the
            // connect loop should break promptly. If it doesn't finish in
            // time (e.g. stuck inside azalea internals), we abandon the
            // join — the OS will clean up when the process exits.
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = handle.join();
                let _ = tx.send(());
            });
            match rx.recv_timeout(std::time::Duration::from_secs(3)) {
                Ok(()) => tracing::info!("bot thread joined cleanly"),
                Err(_) => tracing::warn!("bot thread did not exit within 3s — abandoning join"),
            }
        }
    }
}

impl App for MinecraftApp {
    /// Called before each `ui` frame; used for non-painting logic such as
    /// requesting repaints and lazy-initialising edit buffers.
    fn logic(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        // Request a repaint once per second as a fallback so the uptime
        // counter stays fresh. State-change-driven repaints (via
        // `ctx.request_repaint()` from the event handler) cover the rest.
        ctx.request_repaint_after(std::time::Duration::from_secs(1));

        // Lazy-init the edit buffers from current config.
        if self.edit_config.is_none() {
            let cfg = self.state.read_config();
            self.edit_config = Some(EditConfig::from(&*cfg));
        }
    }

    /// Main UI rendering entry point (egui 0.34 renamed `update` to `ui`).
    ///
    /// The `ui` parameter already provides a root area; we wrap the content
    /// in a `CentralPanel` via `show_inside` to get the standard background
    /// and margins.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // ── Per-frame language sync ─────────────────────────────────
        // Keep the active i18n language in lock-step with the persisted
        // AppConfig value.  Cheap RwLock write, only fires when the value
        // actually changes (e.g. after a fresh config load on startup or
        // when the user applies edits).  The settings panel also calls
        // `i18n::set` directly when the dropdown changes so the new
        // language applies on the next frame without a reconnect.
        let cfg_lang = self.state.read_config().language;
        if crate::ui::i18n::current() != cfg_lang {
            crate::ui::i18n::set(cfg_lang);
        }

        egui::CentralPanel::default().show_inside(ui, |ui| {
            ui.heading(crate::ui::i18n::tr(crate::ui::i18n::TextKey::AppTitle));
            ui.separator();

            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.collapsing(
                    crate::ui::i18n::tr(crate::ui::i18n::TextKey::Settings),
                    |ui| {
                        if let Some(ref mut edit) = self.edit_config {
                            let connect_clicked = settings::settings_panel(ui, &self.state, edit);

                            if connect_clicked {
                                // Persist edits before connecting.
                                edit.apply(&self.state);
                                self.connect_bot();
                            }
                        }
                    },
                );

                ui.collapsing(
                    crate::ui::i18n::tr(crate::ui::i18n::TextKey::Status),
                    |ui| {
                        status::status_panel(ui, &self.state);
                    },
                );

                ui.collapsing(
                    crate::ui::i18n::tr(crate::ui::i18n::TextKey::McpConfig),
                    |ui| {
                        if let Some(ref edit) = self.edit_config {
                            mcp_config::mcp_config_panel(ui, edit);
                        }
                    },
                );
            });
        });
    }
}
