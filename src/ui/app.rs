//! Application shell: window setup, layout, dispatch.
//!
//! [`MinecraftApp`] implements [`eframe::App`] and renders the settings
//! and status panels inside a central layout.  It requests a repaint once
//! per second as a fallback so live state changes (bot connection, world
//! snapshot, chat messages) are reflected in the UI without a manual
//! refresh; state-change-driven repaints (via `ctx.request_repaint()` from
//! the bot event handlers) refresh the UI immediately in between.
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

use crate::bot::spawn::{join_with_timeout, spawn_bot_connection};
use crate::channel::{BotCommandReceiver, BotCommandSender};
use crate::config::{AppConfig, McpTransport};
use crate::i18n::Language;
use crate::state::SharedState;
use crate::ui::{mcp_config, preview, settings, status};

/// Main egui application shell.
pub struct MinecraftApp {
    /// Shared state accessed lock-free for the world snapshot,
    /// and with short-lived read locks for config and stats.
    state: Arc<SharedState>,
    /// Clone of the command channel sender, passed into
    /// [`crate::bot::connection::ConnectionManager::connect`] so compound
    /// ops (e.g. `Act::Mine`) can issue sub-commands through the executor.
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
    /// Handle to the MCP server OS thread. Joined on Drop after triggering
    /// [`SharedState::trigger_shutdown`] so the stdio/HTTP transport exits
    /// gracefully instead of being killed mid-request.
    mcp_handle: Option<JoinHandle<()>>,
    /// Local edit buffers for the settings panel.  Initialised from
    /// [`SharedState`] config on first frame.
    edit_config: Option<EditConfig>,
    /// Cached language from the last frame; lets us avoid a per-frame
    /// `read_config` acquisition just to synchronise `i18n::current()`.
    last_language: Language,
    /// Texture handle for the world-view preview panel. Persisted across
    /// frames so we don't re-upload the same PNG every redraw.
    preview_texture: Option<egui::TextureHandle>,
    /// Rebuild key for the preview texture: the `(snapshot_seq,
    /// annotation_json)` pair the texture was last built from (M-11). When
    /// the cached render's key differs, we rebuild the texture; otherwise we
    /// reuse it (saves a base64 decode + PNG decode every frame). Keying on
    /// `snapshot_seq` as well as the annotation catches two 500ms snapshot
    /// builds sharing one second whose annotations coincide but whose PNGs
    /// differ.
    preview_last_key: Option<preview::PreviewRebuildKey>,
    /// Cached chat messages for the status panel (L-20): re-cloned only when
    /// the chat cursor advances, not every frame.
    chat_cache: status::ChatCache,
    /// Cached MCP-client JSON for the MCP Config panel (L-19): rebuilt only
    /// when a JSON-affecting edit field changes. Lazy-initialised from the
    /// first frame's `EditConfig`.
    mcp_config_cache: Option<mcp_config::McpConfigCache>,
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
    pub fly_timeout_secs: u64,
    /// Bearer token presented by MCP clients over HTTP auth.
    pub mcp_token: String,
    /// Require Bearer-token auth for the HTTP transport (mirrors
    /// [`AppConfig::mcp_auth_enabled`]).
    pub mcp_auth_enabled: bool,
    /// Transport mechanism the MCP server uses to talk to clients.
    pub mcp_transport: McpTransport,
    /// Per-field dirty flags (M-8): which fields the user actually edited
    /// since the buffer was initialised. [`EditConfig::apply`] merges ONLY
    /// the dirty fields, so agent-driven changes made through the
    /// `update_settings` MCP tool are never silently rolled back by the
    /// stale buffer.
    ///
    /// The UI language deliberately has NO buffer field (M-9): the settings
    /// panel binds its dropdown directly to `config.language`, and
    /// [`sync_language_from_config`] is the single writer for
    /// `i18n::current()`.
    pub dirty: EditConfigDirty,
}

/// Per-field dirty flags for the settings-panel edit buffer (M-8).
///
/// Each settings widget marks its field dirty when the user actually edits
/// it (egui's [`egui::Response::changed`]). [`EditConfig::apply`] then
/// merges only the dirty fields into a fresh `state.read_config().clone()`,
/// and clears all flags after a successful apply.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EditConfigDirty {
    pub mc_address: bool,
    pub mc_port: bool,
    pub ai_username: bool,
    pub mcp_address: bool,
    pub mcp_port: bool,
    pub task_name: bool,
    pub chunk_scan_radius: bool,
    pub block_perception_radius: bool,
    pub snapshot_interval_ms: bool,
    pub reconnect_initial_delay_ms: bool,
    pub reconnect_max_delay_ms: bool,
    pub command_timeout_secs: bool,
    pub fly_timeout_secs: bool,
    pub mcp_token: bool,
    pub mcp_auth_enabled: bool,
    pub mcp_transport: bool,
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
            fly_timeout_secs: cfg.fly_timeout_secs,
            mcp_token: cfg.mcp_token.clone(),
            mcp_auth_enabled: cfg.mcp_auth_enabled,
            mcp_transport: cfg.mcp_transport,
            dirty: EditConfigDirty::default(),
        }
    }
}

impl EditConfig {
    /// Write the edited values back into [`SharedState`] config.
    ///
    /// Merges ONLY the fields the user actually edited (their dirty flags
    /// are set by the settings widgets via [`egui::Response::changed`])
    /// into a fresh clone of the current config — untouched fields keep
    /// whatever the config currently holds, so agent-driven changes made
    /// through `update_settings` are never rolled back by the stale buffer
    /// (M-8).
    ///
    /// Validates the resulting [`AppConfig`] before applying it; on
    /// validation failure returns the error message and leaves the stored
    /// config untouched so the user can correct the invalid field. On
    /// success all dirty flags are cleared.
    pub(crate) fn apply(&mut self, state: &SharedState) -> Result<(), String> {
        // Build the candidate config from a clone of the current values, then
        // validate before taking the write lock. The window between read and
        // write is benign here because `apply` is the sole UI writer of config
        // (invoked only from the UI thread on Connect click).
        let mut new_config = state.read_config().clone();
        if self.dirty.mc_address {
            new_config.mc_address = self.mc_address.clone();
        }
        if self.dirty.mc_port {
            new_config.mc_port = self.mc_port;
        }
        if self.dirty.ai_username {
            new_config.ai_username = self.ai_username.clone();
        }
        if self.dirty.mcp_address {
            new_config.mcp_address = self.mcp_address.clone();
        }
        if self.dirty.mcp_port {
            new_config.mcp_port = self.mcp_port;
        }
        if self.dirty.task_name {
            new_config.task_name = self.task_name.clone();
        }
        if self.dirty.chunk_scan_radius {
            new_config.chunk_scan_radius = self.chunk_scan_radius;
        }
        if self.dirty.block_perception_radius {
            new_config.block_perception_radius = self.block_perception_radius;
        }
        if self.dirty.snapshot_interval_ms {
            new_config.snapshot_interval_ms = self.snapshot_interval_ms;
        }
        if self.dirty.reconnect_initial_delay_ms {
            new_config.reconnect_initial_delay_ms = self.reconnect_initial_delay_ms;
        }
        if self.dirty.reconnect_max_delay_ms {
            new_config.reconnect_max_delay_ms = self.reconnect_max_delay_ms;
        }
        if self.dirty.command_timeout_secs {
            new_config.command_timeout_secs = self.command_timeout_secs;
        }
        if self.dirty.fly_timeout_secs {
            new_config.fly_timeout_secs = self.fly_timeout_secs;
        }
        if self.dirty.mcp_token {
            new_config.mcp_token = self.mcp_token.clone();
        }
        if self.dirty.mcp_auth_enabled {
            new_config.mcp_auth_enabled = self.mcp_auth_enabled;
        }
        if self.dirty.mcp_transport {
            new_config.mcp_transport = self.mcp_transport;
        }

        new_config.validate()?;
        state.update_config(|cfg| {
            *cfg = new_config;
        });
        // Only a successful apply clears the dirty flags — a rejected edit
        // must keep them so the user can correct the invalid field.
        self.dirty = EditConfigDirty::default();
        Ok(())
    }
}

/// Single writer for `i18n::current()` on the UI path (M-9).
///
/// Called once per frame from [`App`::ui]: it synchronises the active i18n
/// language with the persisted [`AppConfig::language`] and caches the
/// last-seen language so the per-frame `read_config` acquisition is skipped
/// once the language is stable.
///
/// The settings panel's Language dropdown binds DIRECTLY to
/// `config.language` (no edit-buffer copy) and the `update_settings` MCP
/// tool writes `config.language` too; this function is the only place that
/// calls `i18n::set`, so those two writers can never fight (previously the
/// panel re-applied a stale edit-buffer language every frame, permanently
/// overriding an agent-driven language change).
fn sync_language_from_config(state: &SharedState, last_language: &mut Language) {
    let cfg_lang = state.read_config().language;
    if *last_language != cfg_lang {
        *last_language = cfg_lang;
        crate::i18n::set(cfg_lang);
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
        let initial_lang = state.read_config().language;
        Self {
            state,
            sender,
            command_receiver,
            egui_ctx: None,
            mcp_handle: Some(mcp_handle),
            edit_config: None,
            last_language: initial_lang,
            preview_texture: None,
            preview_last_key: None,
            chat_cache: status::ChatCache::new(),
            mcp_config_cache: None,
        }
    }

    /// Start the bot connection on a dedicated OS thread.
    ///
    /// We spawn a new thread (rather than using `tokio::spawn`) because
    /// azalea's `ClientBuilder::start` internally creates a `LocalSet`
    /// which is `!Send`. The thread body itself lives in
    /// [`crate::bot::spawn::spawn_bot_connection`] so the headless
    /// supervisor and the `connect_bot` MCP tool can reuse it without UI.
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
        // spawning a new one. The previous attempt may have finished (and
        // called `clear_connecting` in its tail) but the `JoinHandle` is
        // still parked in `SharedState`; without this step we'd leak the
        // helper thread that azalea's runtime was using.
        if let Some(prev) = self.state.take_bot_thread_handle() {
            match join_with_timeout(prev, Duration::from_secs(1)) {
                Ok(()) => tracing::debug!("previous bot thread joined cleanly"),
                Err(_) => {
                    tracing::warn!("previous bot thread did not exit within 1s — abandoning join")
                }
            }
        }

        let state = Arc::clone(&self.state);
        let receiver = Arc::clone(&self.command_receiver);
        let sender = self.sender.clone();
        let egui_ctx = self.egui_ctx.clone();

        if let Err(e) = spawn_bot_connection(state, receiver, sender, egui_ctx) {
            tracing::error!(error = %e, "Failed to spawn bot connection thread");
            self.state
                .set_last_error(format!("Failed to spawn bot connection thread: {e}"));
            self.state.clear_connecting();
        }
    }
}

impl Drop for MinecraftApp {
    fn drop(&mut self) {
        // Signal the bot to stop retrying and let the connection thread
        // exit cleanly when the window is closed.
        self.state.request_disconnect();

        // Trigger MCP server graceful shutdown so the stdio/HTTP transport
        // returns promptly. After this, `serve_http`'s `with_graceful_shutdown`
        // future resolves and `serve_stdio`'s `tokio::select!` takes the
        // shutdown branch — both should return in milliseconds.
        self.state.trigger_shutdown();

        if let Some(handle) = self.state.take_bot_thread_handle() {
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
        // `sync_language_from_config` is the single writer for
        // `i18n::current()` (M-9): the settings panel and the
        // `update_settings` MCP tool both write `config.language`, and this
        // per-frame sync applies the change on the next frame. The cached
        // `last_language` avoids a `read_config` RwLock acquisition every
        // frame once the language is stable.
        sync_language_from_config(&self.state, &mut self.last_language);

        egui::CentralPanel::default().show_inside(ui, |ui| {
            ui.heading(crate::i18n::tr(crate::i18n::TextKey::AppTitle));
            ui.separator();

            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.collapsing(crate::i18n::tr(crate::i18n::TextKey::Settings), |ui| {
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
                });

                ui.collapsing(crate::i18n::tr(crate::i18n::TextKey::Status), |ui| {
                    status::status_panel(ui, &self.state, &mut self.chat_cache);
                });

                ui.collapsing(crate::i18n::tr(crate::i18n::TextKey::Preview), |ui| {
                    preview::preview_panel(
                        ui,
                        &self.state,
                        &mut self.preview_texture,
                        &mut self.preview_last_key,
                    );
                });

                ui.collapsing(crate::i18n::tr(crate::i18n::TextKey::McpConfig), |ui| {
                    if let Some(ref edit) = self.edit_config {
                        let cache = self
                            .mcp_config_cache
                            .get_or_insert_with(|| mcp_config::McpConfigCache::new(edit));
                        mcp_config::mcp_config_panel(ui, edit, cache);
                    }
                });
            });
        });
    }
}

// Tests for the bot-connection spawn helper (incl. `join_with_timeout`,
// which used to live in this file) are in `src/bot/spawn.rs::tests`.

// ═══════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::state::SharedState;

    /// `EditConfig::apply` persists every field the user actually edited
    /// (dirty flags set) into `SharedState`.
    ///
    /// Rewritten for the M-8 dirty-flag API: `apply` merges only fields
    /// whose dirty flag is set, so the test now marks each edited field
    /// dirty (exactly what the settings widgets do via
    /// `Response::changed`).
    #[test]
    fn test_edit_config_apply_persists_changes() {
        let state = SharedState::new(AppConfig::default());
        let mut edit = EditConfig::from(&state.read_config().clone());
        edit.mc_address = "mc.example.com".into();
        edit.mc_port = 25566;
        edit.ai_username = "Robot".into();
        edit.mcp_port = 9011;
        edit.command_timeout_secs = 42;
        // Mark the edited fields dirty (mirrors the settings widgets).
        edit.dirty.mc_address = true;
        edit.dirty.mc_port = true;
        edit.dirty.ai_username = true;
        edit.dirty.mcp_port = true;
        edit.dirty.command_timeout_secs = true;

        edit.apply(&state).expect("valid edit should apply");

        let cfg = state.read_config();
        assert_eq!(cfg.mc_address, "mc.example.com");
        assert_eq!(cfg.mc_port, 25566);
        assert_eq!(cfg.ai_username, "Robot");
        assert_eq!(cfg.mcp_port, 9011);
        assert_eq!(cfg.command_timeout_secs, 42);
        // Dirty flags are cleared after a successful apply.
        assert_eq!(edit.dirty, EditConfigDirty::default());
    }

    /// An invalid edit (empty `mc_address`) is rejected and leaves the
    /// stored config untouched.
    #[test]
    fn test_edit_config_apply_rejects_invalid_and_leaves_config() {
        let state = SharedState::new(AppConfig::default());
        let mut edit = EditConfig::from(&state.read_config().clone());
        edit.mc_address.clear();
        edit.dirty.mc_address = true;

        assert!(edit.apply(&state).is_err());
        // The original config is unchanged.
        assert!(!state.read_config().mc_address.is_empty());
        // Failed validation must NOT clear the dirty flag — the user still
        // needs to correct the field.
        assert!(edit.dirty.mc_address);
    }

    /// M-8: `EditConfig::apply` must not roll back agent-driven config
    /// changes for fields the user did not edit. The buffer is initialised
    /// once from config; when the agent later changes `mc_port` via
    /// `update_settings`, a user edit to ONLY `mc_address` must leave
    /// `mc_port` untouched (previously `apply` overwrote every field from
    /// the stale buffer, silently reverting the agent's change).
    #[test]
    fn test_apply_does_not_roll_back_unedited_agent_change() {
        let state = SharedState::new(AppConfig::default());
        // Buffer initialised from the config at startup.
        let mut edit = EditConfig::from(&state.read_config().clone());
        // Agent changes mc_port via update_settings AFTER the buffer was made.
        state.update_config(|cfg| cfg.mc_port = 25566);
        // User edits only mc_address in the buffer (dirty flag set).
        edit.mc_address = "mc.example.com".into();
        edit.dirty.mc_address = true;

        edit.apply(&state).expect("valid edit should apply");

        let cfg = state.read_config();
        assert_eq!(cfg.mc_address, "mc.example.com", "edited field applies");
        assert_eq!(cfg.mc_port, 25566, "unedited field must survive apply");
        // Dirty flags are cleared after a successful apply.
        assert!(!edit.dirty.mc_address);
    }

    /// M-9: an agent-driven `update_settings(language=zh_cn)` must not be
    /// reverted by the settings panel. `sync_language_from_config` (called
    /// once per frame from `ui()`) is the single writer for
    /// `i18n::current()`; the panel no longer owns a language buffer and
    /// never calls `i18n::set` with a stale value.
    #[test]
    fn test_language_change_via_config_not_fought_by_panel() {
        crate::i18n::set(Language::En);
        let state = SharedState::new(AppConfig::default());
        // Agent changed the language via update_settings.
        state.update_config(|cfg| cfg.language = Language::ZhCn);

        let mut last_language = Language::En;
        // Frame 1: the app syncs i18n from config (single writer).
        sync_language_from_config(&state, &mut last_language);
        assert_eq!(crate::i18n::current(), Language::ZhCn);
        assert_eq!(last_language, Language::ZhCn);

        // Frame 2 (and every subsequent): the sync no-ops (already in sync)
        // and the panel cannot fight it — there is no edit-buffer language
        // to re-apply, so i18n stays ZhCn.
        sync_language_from_config(&state, &mut last_language);
        assert_eq!(crate::i18n::current(), Language::ZhCn);

        // Restore the global i18n state for other tests.
        crate::i18n::set(Language::En);
    }
}
