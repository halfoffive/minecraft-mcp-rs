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
use std::time::Duration;

use eframe::App;
use egui::Context;

use crate::bot::connection::ConnectionManager;
use crate::channel::{BotCommandReceiver, BotCommandSender};
use crate::config::{AppConfig, McpTransport};
use crate::state::SharedState;
use crate::ui::i18n::Language;
use crate::ui::{mcp_config, settings, status};

/// Join a [`JoinHandle`] on a background OS thread, bounded by `timeout`.
///
/// `Drop::join` would block indefinitely if the spawned task is wedged
/// inside a third-party runtime, which would freeze the window close. To
/// keep the UI responsive we move the `join()` into a helper thread and
/// wait on an `mpsc` channel with a deadline:
///
/// - `Ok(())` — the handle finished (cleanly or by panic) within
///   `timeout`; the helper thread has already consumed the
///   `JoinHandle`.
/// - `Err(timeout)` — the timeout fired first; the helper thread is left
///   running with the `JoinHandle` in flight. When the process eventually
///   exits, the OS reclaims the thread. This is acceptable here because
///   the handle is purely a UI-side bookkeeping reference; the actual
///   runtime inside the thread is supposed to exit via
///   `state.request_disconnect()` + `cancel_token`.
fn join_with_timeout(handle: JoinHandle<()>, timeout: Duration) -> Result<(), Duration> {
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        let _ = handle.join();
        let _ = tx.send(());
    });
    match rx.recv_timeout(timeout) {
        Ok(()) => Ok(()),
        Err(_) => Err(timeout),
    }
}

/// Main egui application shell.
pub struct MinecraftApp {
    /// Shared state accessed lock-free for the world snapshot,
    /// and with short-lived read locks for config and stats.
    state: Arc<SharedState>,
    /// Clone of the command channel sender, passed into
    /// [`ConnectionManager::connect`] so compound ops (e.g. `Act::Mine`) can
    /// issue sub-commands through the executor.
    sender: BotCommandSender,
    /// Shared command receiver slot, passed to the bot connection task so it
    /// can process commands from the MCP server while connected. The receiver
    /// is leased out to the command executor on `Event::Spawn`.
    command_receiver: Arc<std::sync::Mutex<Option<BotCommandReceiver>>>,
    /// Most recent egui context, refreshed each frame in `logic` so the bot
    /// connection thread can call `request_repaint` and have the UI refresh
    /// immediately on state changes (spawn, disconnect, death, etc.) instead
    /// of waiting for the 1-second fallback repaint. `None` until the first
    /// frame has run.
    egui_ctx: Option<egui::Context>,
    /// Handle to the bot-connection OS thread (if running). Joined on Drop
    /// so the process exits cleanly when the window closes.
    bot_thread: Option<JoinHandle<()>>,
    /// Handle to the MCP server OS thread. Joined on Drop after triggering
    /// [`SharedState::trigger_shutdown`] so the stdio/HTTP transport exits
    /// gracefully instead of being killed mid-request.
    mcp_handle: Option<JoinHandle<()>>,
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
    ///
    /// Validates the resulting [`AppConfig`] before applying it; on
    /// validation failure returns the error message and leaves the stored
    /// config untouched so the user can correct the invalid field.
    pub(crate) fn apply(&self, state: &SharedState) -> Result<(), String> {
        // Build the candidate config from a clone of the current values, then
        // validate before taking the write lock. The window between read and
        // write is benign here because `apply` is the sole writer of config
        // (invoked only from the UI thread on Connect click).
        let mut new_config = state.read_config().clone();
        new_config.mc_address = self.mc_address.clone();
        new_config.mc_port = self.mc_port;
        new_config.ai_username = self.ai_username.clone();
        new_config.mcp_address = self.mcp_address.clone();
        new_config.mcp_port = self.mcp_port;
        new_config.task_name = self.task_name.clone();
        new_config.chunk_scan_radius = self.chunk_scan_radius;
        new_config.block_perception_radius = self.block_perception_radius;
        new_config.snapshot_interval_ms = self.snapshot_interval_ms;
        new_config.reconnect_initial_delay_ms = self.reconnect_initial_delay_ms;
        new_config.reconnect_max_delay_ms = self.reconnect_max_delay_ms;
        new_config.command_timeout_secs = self.command_timeout_secs;
        new_config.mcp_token = self.mcp_token.clone();
        new_config.mcp_transport = self.mcp_transport;
        new_config.language = self.language;

        new_config.validate()?;
        state.update_config(|cfg| {
            *cfg = new_config;
        });
        Ok(())
    }
}

impl MinecraftApp {
    /// Create a new [`MinecraftApp`].
    pub fn new(
        state: Arc<SharedState>,
        sender: BotCommandSender,
        command_receiver: Arc<std::sync::Mutex<Option<BotCommandReceiver>>>,
        mcp_handle: JoinHandle<()>,
    ) -> Self {
        // `sender` is cloned into the bot connection thread on each Connect
        // click (see `connect_bot`) so compound ops can issue sub-commands.
        // The MCP server thread holds its own clone and is the primary
        // consumer of the command channel.
        Self {
            state,
            sender,
            command_receiver,
            egui_ctx: None,
            bot_thread: None,
            mcp_handle: Some(mcp_handle),
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

        // P1-#14: defensively join any previously-spawned bot thread before
        // overwriting `self.bot_thread`. The previous attempt may have
        // finished (and called `clear_connecting` in its tail) but the
        // `JoinHandle` is still parked in this field; without this step
        // we'd leak the helper thread that azalea's runtime was using.
        if let Some(prev) = self.bot_thread.take() {
            match join_with_timeout(prev, Duration::from_secs(1)) {
                Ok(()) => tracing::debug!("previous bot thread joined cleanly"),
                Err(_) => {
                    tracing::warn!("previous bot thread did not exit within 1s — abandoning join")
                }
            }
        }

        let config = self.state.read_config().clone();
        let state = Arc::clone(&self.state);
        let receiver = Arc::clone(&self.command_receiver);
        let sender = self.sender.clone();
        let egui_ctx = self.egui_ctx.clone();

        let handle = std::thread::Builder::new()
            .name("bot-connection".into())
            .spawn(move || {
                let rt = tokio::runtime::Runtime::new()
                    .expect("Failed to create tokio runtime for bot connection");
                let manager = ConnectionManager::new(config, Arc::clone(&state));

                rt.block_on(async move {
                    if let Err(e) = manager.connect(receiver, egui_ctx, sender).await {
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

        // Trigger MCP server graceful shutdown so the stdio/HTTP transport
        // returns promptly. After this, `serve_http`'s `with_graceful_shutdown`
        // future resolves and `serve_stdio`'s `tokio::select!` takes the
        // shutdown branch — both should return in milliseconds.
        self.state.trigger_shutdown();

        if let Some(handle) = self.bot_thread.take() {
            // Try to join with a 3-second timeout to avoid hanging the UI
            // thread. The bot thread runs its own tokio runtime; with
            // `disconnect_requested` set and the cancel token tripped, the
            // connect loop should break promptly. If it doesn't finish in
            // time (e.g. stuck inside azalea internals), we abandon the
            // join — the OS will clean up when the process exits.
            match join_with_timeout(handle, Duration::from_secs(3)) {
                Ok(()) => tracing::info!("bot thread joined cleanly"),
                Err(_) => tracing::warn!("bot thread did not exit within 3s — abandoning join"),
            }
        }

        if let Some(handle) = self.mcp_handle.take() {
            // Same timeout pattern as the bot thread for consistency and
            // safety. Graceful shutdown should return in milliseconds, but
            // if a misbehaving MCP client holds a connection open (HTTP) or
            // the rmcp transport doesn't observe the select branch promptly
            // (stdio), we abandon the join after 3s rather than hanging the
            // window close.
            match join_with_timeout(handle, Duration::from_secs(3)) {
                Ok(()) => tracing::info!("mcp thread joined cleanly"),
                Err(_) => tracing::warn!("mcp thread did not exit within 3s — abandoning join"),
            }
        }
    }
}

impl App for MinecraftApp {
    /// Called before each `ui` frame; used for non-painting logic such as
    /// requesting repaints and lazy-initialising edit buffers.
    fn logic(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        // Cache the egui context so `connect_bot` can hand a real `Context`
        // to the bot connection thread, which in turn injects it into
        // `BotState` so the event handlers can call `request_repaint` for
        // immediate UI refreshes on spawn / disconnect / death.
        self.egui_ctx = Some(ctx.clone());

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
                                // Persist edits before connecting. If
                                // validation fails, surface the error via
                                // `last_error` (shown in the Status panel)
                                // and skip connecting so the user can fix
                                // the invalid field.
                                match edit.apply(&self.state) {
                                    Ok(()) => self.connect_bot(),
                                    Err(e) => self.state.set_last_error(e),
                                }
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

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// `join_with_timeout` returns `Ok(())` when the handle finishes before
    /// the deadline.
    #[test]
    fn test_join_with_timeout_completes_before_deadline() {
        let handle = std::thread::spawn(|| {
            // Short, well-bounded work.
            let mut acc: u64 = 0;
            for i in 0..1000 {
                acc = acc.wrapping_add(i);
            }
            std::hint::black_box(acc);
        });
        let result = join_with_timeout(handle, Duration::from_secs(2));
        assert!(
            result.is_ok(),
            "fast thread should join within 2s: {result:?}"
        );
    }

    /// `join_with_timeout` returns `Err(timeout)` when the handle is still
    /// running past the deadline. We use a `std::thread::park` + a manual
    /// unpark from the test thread to make the test deterministic and not
    /// depend on `recv_timeout` being exactly equal to wall time.
    #[test]
    fn test_join_with_timeout_abandons_when_thread_hangs() {
        let parked_thread = std::thread::Builder::new()
            .name("parked-test-thread".into())
            .spawn(|| {
                // Park indefinitely — the helper thread that holds the
                // JoinHandle is then blocked on `join()` until this
                // unpark. We never unpark, so the helper remains stuck
                // for the full test duration.
                std::thread::park();
            })
            .expect("spawn parked thread");

        // The actual `JoinHandle` we hand to `join_with_timeout` is the
        // parked thread's own handle. To make the test even more robust
        // we wrap it in a long-sleeping thread that owns the parked
        // thread's JoinHandle and blocks on it; this mirrors the
        // "stalled third-party runtime" scenario the helper exists for.
        let parked_handle = parked_thread;
        let handle = std::thread::spawn(move || {
            let _ = parked_handle.join();
        });

        // 100ms is comfortably above scheduling jitter but short enough
        // to keep the test snappy.
        let result = join_with_timeout(handle, Duration::from_millis(100));
        assert!(result.is_err(), "expected timeout, got {result:?}");
    }
}
