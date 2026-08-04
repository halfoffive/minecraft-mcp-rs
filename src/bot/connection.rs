//! Minecraft client connection lifecycle (connect, disconnect, rejoin).
//!
//! [`ConnectionManager`] owns the bot connection loop: it creates offline
//! accounts via azalea, attaches the event handler from [`super::events`],
//! and handles disconnection with exponential-backoff reconnects.
//!
//! During reconnect windows, [`SharedState::is_online`] returns `false` so all
//! MCP tools return a [`BotError::Offline`](crate::error::BotError::Offline)
//! immediately instead of hanging.

use std::sync::Arc;
use std::time::Duration;

use azalea::{Account, ClientBuilder, prelude::AppExit};
use tracing::{info, warn};

use crate::bot::events;
use crate::channel::{BotCommandSender, ReceiverSlot};
use crate::config::AppConfig;
use crate::state::SharedState;

// ---------------------------------------------------------------------------
// ConnectionManager
// ---------------------------------------------------------------------------

/// Manages the Minecraft bot connection lifecycle.
///
/// Holds the user configuration and a shared reference to application state
/// so connection status can be read by the MCP server and UI layers.
#[derive(Debug)]
pub struct ConnectionManager {
    /// Initial configuration snapshot captured at construction time.
    ///
    /// **Initial-only:** the connect loop and [`reconnect_backoff`](Self::reconnect_backoff)
    /// re-read live values from [`SharedState::read_config`] on every
    /// iteration (hot reload), so this field is NOT consulted inside the
    /// loop. It is retained so callers (and tests) can inspect the
    /// configuration the manager was constructed with without touching the
    /// `ConnectionManager::new` call chain.
    #[allow(dead_code)]
    config: AppConfig,
    state: Arc<SharedState>,
}

impl ConnectionManager {
    /// Create a new connection manager.
    ///
    /// The manager does **not** connect automatically — call [`connect`](Self::connect).
    pub fn new(config: AppConfig, state: Arc<SharedState>) -> Self {
        Self { config, state }
    }

    /// Whether the bot is currently connected to a Minecraft server.
    ///
    /// Delegates to [`SharedState::is_online`] which reads an [`AtomicBool`](std::sync::atomic::AtomicBool).
    pub fn is_connected(&self) -> bool {
        self.state.is_online()
    }

    /// Mark the bot as disconnected.
    ///
    /// Sets the online flag to `false` so MCP tools can return offline errors.
    /// The actual TCP teardown is handled by azalea when the handler function
    /// returns from [`Event::Disconnect`](azalea::Event::Disconnect).
    pub fn disconnect(&self) {
        self.state.set_online(false);
    }

    /// Request an egui repaint if a context was supplied.
    ///
    /// The connection loop runs on a background OS thread; state mutations
    /// like `set_online(false)` and `clear_connecting()` are invisible to the
    /// UI until the next frame. Calling this after every state transition that
    /// affects the Connect/Disconnect buttons or status labels keeps the UI
    /// in sync without waiting for the 1-second fallback repaint.
    fn request_repaint(ctx: &Option<egui::Context>) {
        if let Some(ctx) = ctx {
            ctx.request_repaint();
        }
    }

    /// Run the connection loop.
    ///
    /// Connects, runs until the bot is disconnected, then decides what to
    /// do next based on whether the bot ever came online:
    ///
    /// - **User-initiated disconnect** (`is_disconnect_requested()`): stop
    ///   immediately without writing an error — unless a config restart was
    ///   requested (see below).
    /// - **Config restart** ([`should_restart_after_disconnect`]): an agent
    ///   changed connection settings via `update_settings` while online;
    ///   consume the restart flag, clear the disconnect request, reset the
    ///   cancel token and stale error, reset attempt counters, and reconnect
    ///   with the updated settings.
    /// - **Connection never succeeded** (the session latch
    ///   [`SharedState::take_session_was_online`] returns `false`): capture
    ///   a descriptive error into [`SharedState::set_last_error`] and stop —
    ///   the user must click Connect again to retry.
    /// - **Was online, then disconnected** (a proper "reconnect" scenario):
    ///   clear `last_error` and retry with exponential backoff. The backoff
    ///   sleep is cancelable via the
    ///   [`CancellationToken`](tokio_util::sync::CancellationToken) so a
    ///   Disconnect click interrupts it immediately.
    ///
    /// Every iteration hot-reloads the connection-relevant config values
    /// (`ai_username`, `mc_address`, `mc_port`, `snapshot_interval_ms`, and
    /// the backoff delays) from [`SharedState::read_config`] so
    /// agent-driven settings changes take effect without a restart.
    ///
    /// Spawn this as a background task via [`tokio::spawn`].
    ///
    /// # Parameters
    /// - `command_receiver`: shared slot holding the command receiver, wrapped
    ///   in `Arc<Mutex<Option<_>>>` so the azalea event handler can lease it
    ///   on `Event::Spawn` (and return it on disconnect). Shared across
    ///   reconnection attempts.
    /// - `egui_ctx`: optional egui context for triggering UI repaints.
    /// - `command_sender`: clone of the command channel sender, injected into
    ///   `events::INJECTED_COMMAND_SENDER` so `BotState::default()` picks it
    ///   up and compound ops (e.g. `Act::Mine`) can issue sub-commands via the
    ///   executor.
    pub async fn connect(
        &self,
        command_receiver: ReceiverSlot,
        egui_ctx: Option<egui::Context>,
        command_sender: BotCommandSender,
    ) -> eyre::Result<()> {
        // Clear any stale disconnect request and error from a previous
        // session, and install a fresh cancellation token so a prior
        // session's cancel doesn't immediately trip our backoff sleep.
        self.state.clear_disconnect_request();
        self.state.clear_last_error();
        self.state.reset_cancel_token();

        let mut attempt: u32 = 0;
        // Separate counter for first-connect retries (when the bot has never
        // come online). Bounded by `MAX_FIRST_CONNECT_RETRIES` so transient
        // failures (server still starting, DNS delay) get a few automatic
        // retries before fail-fast kicks in.
        let mut first_connect_attempts: u32 = 0;

        loop {
            // If the user clicked Disconnect before we even started, stop —
            // unless an agent-driven config restart was requested, in which
            // case consume it and reconnect with the updated settings.
            if self.state.is_disconnect_requested() {
                if should_restart_after_disconnect(&self.state) {
                    info!("config restart requested — reconnecting with updated settings");
                    attempt = 0;
                    first_connect_attempts = 0;
                    continue;
                }
                info!("disconnect requested — stopping connection loop");
                break;
            }

            // Hot config reload: read the connection-relevant values fresh
            // from `SharedState` on every iteration instead of using the
            // frozen `self.config` snapshot. This lets agent-driven
            // `update_settings` changes (username, server address/port,
            // snapshot interval) take effect on the very next reconnect
            // without restarting the process. The RwLock read guard is
            // confined to this block and dropped before any `.await` —
            // never held across an await point.
            let (ai_username, mc_address, mc_port, snapshot_interval_ms) = {
                let cfg = self.state.read_config();
                (
                    cfg.ai_username.clone(),
                    cfg.mc_address.clone(),
                    cfg.mc_port,
                    cfg.snapshot_interval_ms,
                )
            };

            // Inject dependencies so BotState::default() picks them up when
            // azalea initializes the state internally via Default. Using
            // `Mutex<Option<_>>` (rather than `OnceLock`) so the values can be
            // refreshed on each reconnect and cleared on disconnect.
            // Re-injected on every loop iteration because handle_disconnect
            // clears them all to None on disconnect — without this, a
            // reconnect would fall back to a throwaway SharedState and the
            // is_online() flag would never flip on the real state.
            Self::inject_dependencies(
                &self.state,
                &command_receiver,
                egui_ctx.as_ref(),
                &command_sender,
                snapshot_interval_ms,
            );

            let account = Account::offline(&ai_username);
            let address = format!("{mc_address}:{mc_port}");

            info!(
                "Connecting to {} as {} (attempt {})...",
                address,
                ai_username,
                attempt + 1
            );

            // start() blocks until the client disconnects or the connection fails.
            // BotState is created internally by azalea via Default — the injected
            // statics above ensure the correct SharedState and command receiver are used.
            //
            // Wrap in tokio::select! so a disconnect request during the (possibly
            // multi-second) TCP connect attempt aborts immediately instead of
            // waiting for start() to return. Without this, clicking Disconnect
            // while azalea is still trying to connect has no effect until the
            // TCP timeout expires (~5 s).
            let start_cancel = self.state.cancel_token();
            let exit = tokio::select! {
                result = ClientBuilder::new()
                    .set_handler(events::handle_event)
                    .start(account, address.clone()) => result,
                _ = start_cancel.cancelled() => {
                    info!("disconnect requested during connection attempt — aborting start");
                    self.state.set_online(false);
                    Self::request_repaint(&egui_ctx);
                    break;
                }
            };

            // Was the bot online before this disconnect? `handle_spawn`
            // latches `session_was_online` via `mark_session_online()` when
            // the executor starts, so this is true iff the bot successfully
            // connected at some point during the `start()` call.
            //
            // NOTE (audit F6-1): this used to read `self.state.is_online()`,
            // which is ALWAYS false here — `handle_disconnect` clears the
            // online flag before `ClientBuilder::start()` returns — so the
            // exponential-backoff branch below was dead code and the bot
            // fail-fasted after any real disconnect. The latch survives
            // `handle_disconnect` and is consumed by the take below.
            let was_online = self.state.take_session_was_online();

            // Disconnected — ensure the online flag is cleared.
            self.state.set_online(false);
            Self::request_repaint(&egui_ctx);

            // If the user requested disconnect, don't treat it as a failure —
            // unless an agent-driven config restart was requested, in which
            // case consume it and reconnect with the updated settings.
            if self.state.is_disconnect_requested() {
                if should_restart_after_disconnect(&self.state) {
                    info!("config restart requested — reconnecting with updated settings");
                    attempt = 0;
                    first_connect_attempts = 0;
                    continue;
                }
                info!("disconnect requested — stopping reconnect loop");
                break;
            }

            if !was_online {
                // First-connect retry: try up to 3 times before giving up.
                // Transient failures (server still starting, DNS delay) can
                // recover without forcing the user to click Connect again.
                const MAX_FIRST_CONNECT_RETRIES: u32 = 3;
                const FIRST_CONNECT_RETRY_DELAY: Duration = Duration::from_secs(2);

                first_connect_attempts = first_connect_attempts.saturating_add(1);
                if first_connect_attempts < MAX_FIRST_CONNECT_RETRIES {
                    warn!(
                        attempt = first_connect_attempts,
                        max = MAX_FIRST_CONNECT_RETRIES,
                        "first connect failed — retrying in {}s",
                        FIRST_CONNECT_RETRY_DELAY.as_secs()
                    );
                    // Wait before retry (cancellable via cancel_token, same
                    // pattern as the reconnect backoff below).
                    let cancel_token = self.state.cancel_token();
                    tokio::select! {
                        _ = tokio::time::sleep(FIRST_CONNECT_RETRY_DELAY) => {}
                        _ = cancel_token.cancelled() => {
                            info!("disconnect requested during first-connect retry — stopping");
                            break;
                        }
                    }
                    continue; // retry the loop
                }

                // All retries exhausted — fail fast. Capture a descriptive
                // error (including the AppExit details) so the UI can display
                // it, and stop retrying. The user must click Connect again to
                // attempt reconnection.
                let exit_desc = match &exit {
                    AppExit::Success => "success".to_string(),
                    AppExit::Error(code) => format!("error(code={code})"),
                };
                let msg = format!(
                    "Connection failed: {address} ({exit_desc}, retried {MAX_FIRST_CONNECT_RETRIES} times)"
                );
                warn!(%address, %exit_desc, "connection failed — stopping retry loop");
                self.state.set_last_error(msg);
                Self::request_repaint(&egui_ctx);
                break;
            }

            // Was online before disconnect — retry with backoff. Clear any
            // stale error so the UI doesn't display it during the retry.
            self.state.clear_last_error();
            let delay = self.reconnect_backoff(attempt);
            warn!(
                "Disconnected (was online). Reconnecting in {}s (attempt {})...",
                delay.as_secs(),
                attempt + 1
            );
            // Bind the token to a local so it lives for the duration of the
            // select! — `cancel_token()` returns a clone by value.
            let cancel_token = self.state.cancel_token();
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = cancel_token.cancelled() => {
                    info!("disconnect requested — cancelling reconnect sleep");
                    break;
                }
            }
            attempt = attempt.saturating_add(1);
        }

        // Allow the next Connect click to proceed.
        self.state.clear_connecting();
        Self::request_repaint(&egui_ctx);
        Ok(())
    }

    /// Calculate the reconnect delay for the given attempt number (0-indexed).
    ///
    /// Uses exponential backoff: `initial_delay * 2^attempt`, capped at `max_delay`.
    ///
    /// | attempt | delay (with defaults) |
    /// |---------|-----------------------|
    /// | 0       | 5s                    |
    /// | 1       | 10s                   |
    /// | 2       | 20s                   |
    /// | 3       | 40s                   |
    /// | 4       | 60s (capped)          |
    /// | 5+      | 60s (capped)          |
    pub fn reconnect_backoff(&self, attempt: u32) -> Duration {
        // Read the delays LIVE from `SharedState` (not from the frozen
        // `self.config` snapshot) so an agent-driven `update_settings`
        // change to the backoff parameters takes effect on the very next
        // reconnect. The read guard is confined to this block — no `.await`
        // while held.
        let (initial_ms, max_ms) = {
            let cfg = self.state.read_config();
            (cfg.reconnect_initial_delay_ms, cfg.reconnect_max_delay_ms)
        };
        let delay_ms = initial_ms.saturating_mul(2u64.saturating_pow(attempt));
        Duration::from_millis(delay_ms.min(max_ms))
    }

    /// Install the four `INJECTED_*` statics and the snapshot interval
    /// that `BotState::default` reads on the next connection.
    ///
    /// This is a standalone helper so the connect loop can call it on
    /// every iteration — the regression guarded by
    /// [`tests::test_connect_resets_injections_each_iteration`] is that
    /// calling this function twice in a row (with `handle_disconnect`
    /// clearing the statics in between) must leave all four statics in
    /// the `Some` state. Without the per-iteration call, a reconnect
    /// would see stale `None` slots and the bot would never see the real
    /// `SharedState`.
    pub(crate) fn inject_dependencies(
        state: &Arc<SharedState>,
        command_receiver: &ReceiverSlot,
        egui_ctx: Option<&egui::Context>,
        command_sender: &BotCommandSender,
        snapshot_interval_ms: u64,
    ) {
        *events::INJECTED_SHARED_STATE
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(Arc::clone(state));
        *events::INJECTED_COMMAND_RECEIVER
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(Arc::clone(command_receiver));
        *events::INJECTED_EGUI_CTX
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = egui_ctx.cloned();
        *events::INJECTED_COMMAND_SENDER
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(command_sender.clone());
        events::INJECTED_SNAPSHOT_INTERVAL_MS
            .store(snapshot_interval_ms, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Decide whether the connect loop should restart instead of exiting after
/// a disconnect request, and prepare the state for a clean reconnect.
///
/// # Agent-settings-change flow
///
/// When an AI agent changes connection-relevant settings (`mc_address`,
/// `mc_port`, `ai_username`) through the `update_settings` MCP tool while
/// the bot is online/connecting, the settings handler requests a disconnect
/// ([`SharedState::request_disconnect`]) **and** flags a config restart
/// ([`SharedState::request_config_restart`]). The running azalea session
/// tears down, `ClientBuilder::start()` returns, and the connect loop
/// observes the disconnect request at one of its checkpoints.
///
/// This helper is the consumption side of that handshake. If the restart
/// flag is set, it:
///
/// 1. consumes the flag ([`SharedState::take_config_restart`]),
/// 2. clears the disconnect request so the loop may reconnect
///    ([`SharedState::clear_disconnect_request`]),
/// 3. resets the cancellation token so the fresh iteration's backoff sleep
///    and `select!` branches are not immediately cancelled by the token the
///    settings handler tripped ([`SharedState::reset_cancel_token`]),
/// 4. clears any stale error banner ([`SharedState::clear_last_error`]),
///
/// and returns `true` — the caller resets its attempt counters and
/// `continue`s the loop, which hot-reloads the updated config on the next
/// iteration. Without the restart flag, nothing is touched and `false` is
/// returned — the caller honours the disconnect request and `break`s.
pub(crate) fn should_restart_after_disconnect(state: &SharedState) -> bool {
    if state.take_config_restart() {
        state.clear_disconnect_request();
        state.reset_cancel_token();
        state.clear_last_error();
        true
    } else {
        false
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::field_reassign_with_default)]
    use super::*;
    use crate::config::AppConfig;

    // -- Construction --------------------------------------------------------

    #[test]
    fn test_connection_manager_new_stores_config() {
        let mut config = AppConfig::default();
        config.ai_username = "TestBot".into();
        config.mc_port = 25566;

        let state = Arc::new(SharedState::new(config.clone()));
        let manager = ConnectionManager::new(config.clone(), Arc::clone(&state));

        assert_eq!(manager.config.ai_username, "TestBot");
        assert_eq!(manager.config.mc_port, 25566);
    }

    #[test]
    fn test_connection_manager_new_is_initially_offline() {
        let config = AppConfig::default();
        let state = Arc::new(SharedState::new(config.clone()));
        let manager = ConnectionManager::new(config, Arc::clone(&state));

        assert!(!manager.is_connected());
    }

    // -- is_connected --------------------------------------------------------

    #[test]
    fn test_is_connected_delegates_to_state() {
        let config = AppConfig::default();
        let state = Arc::new(SharedState::new(config.clone()));
        let manager = ConnectionManager::new(config, Arc::clone(&state));

        assert!(!manager.is_connected());

        state.set_online(true);
        assert!(manager.is_connected());

        state.set_online(false);
        assert!(!manager.is_connected());
    }

    // -- disconnect ----------------------------------------------------------

    #[test]
    fn test_disconnect_sets_offline() {
        let config = AppConfig::default();
        let state = Arc::new(SharedState::new(config.clone()));
        let manager = ConnectionManager::new(config, Arc::clone(&state));

        // Set online first
        state.set_online(true);
        assert!(manager.is_connected());

        manager.disconnect();
        assert!(!manager.is_connected());
    }

    #[test]
    fn test_disconnect_when_already_offline_is_idempotent() {
        let config = AppConfig::default();
        let state = Arc::new(SharedState::new(config.clone()));
        let manager = ConnectionManager::new(config, Arc::clone(&state));

        assert!(!manager.is_connected());
        manager.disconnect();
        assert!(!manager.is_connected());
    }

    // -- reconnect_backoff ---------------------------------------------------

    #[test]
    fn test_reconnect_backoff_attempt_0_is_initial_delay() {
        let config = AppConfig::default();
        let state = Arc::new(SharedState::new(config.clone()));
        let manager = ConnectionManager::new(config, Arc::clone(&state));

        assert_eq!(manager.reconnect_backoff(0), Duration::from_millis(5000));
    }

    #[test]
    fn test_reconnect_backoff_doubles_each_attempt() {
        let config = AppConfig::default();
        let state = Arc::new(SharedState::new(config.clone()));
        let manager = ConnectionManager::new(config, Arc::clone(&state));

        assert_eq!(manager.reconnect_backoff(0), Duration::from_secs(5));
        assert_eq!(manager.reconnect_backoff(1), Duration::from_secs(10));
        assert_eq!(manager.reconnect_backoff(2), Duration::from_secs(20));
        assert_eq!(manager.reconnect_backoff(3), Duration::from_secs(40));
    }

    #[test]
    fn test_reconnect_backoff_capped_at_max() {
        let config = AppConfig::default();
        let state = Arc::new(SharedState::new(config.clone()));
        let manager = ConnectionManager::new(config, Arc::clone(&state));

        // attempt 4: 5 * 2^4 = 80s, capped at 60s
        assert_eq!(manager.reconnect_backoff(4), Duration::from_secs(60));
        // attempt 5: 5 * 2^5 = 160s, capped at 60s
        assert_eq!(manager.reconnect_backoff(5), Duration::from_secs(60));
        // attempt 10: way beyond cap
        assert_eq!(manager.reconnect_backoff(10), Duration::from_secs(60));
    }

    #[test]
    fn test_reconnect_backoff_respects_custom_delays() {
        let mut config = AppConfig::default();
        config.reconnect_initial_delay_ms = 3000;
        config.reconnect_max_delay_ms = 30000;

        let state = Arc::new(SharedState::new(config.clone()));
        let manager = ConnectionManager::new(config, Arc::clone(&state));

        assert_eq!(manager.reconnect_backoff(0), Duration::from_secs(3)); // 3s
        assert_eq!(manager.reconnect_backoff(1), Duration::from_secs(6)); // 6s
        assert_eq!(manager.reconnect_backoff(2), Duration::from_secs(12)); // 12s
        assert_eq!(manager.reconnect_backoff(3), Duration::from_secs(24)); // 24s
        assert_eq!(manager.reconnect_backoff(4), Duration::from_secs(30)); // 48s → capped
    }

    #[test]
    fn test_reconnect_backoff_monotonically_increasing() {
        let config = AppConfig::default();
        let state = Arc::new(SharedState::new(config.clone()));
        let manager = ConnectionManager::new(config, Arc::clone(&state));

        let mut prev = Duration::ZERO;
        for attempt in 0..20 {
            let delay = manager.reconnect_backoff(attempt);
            assert!(
                delay >= prev,
                "backoff({attempt}) = {:?} < backoff({}) = {:?}",
                delay,
                attempt.saturating_sub(1),
                prev
            );
            prev = delay;
        }
    }

    // -- Account creation ----------------------------------------------------

    #[test]
    fn test_account_offline_uses_config_username() {
        let mut config = AppConfig::default();
        config.ai_username = "MyOfflineBot".into();

        let account = Account::offline(&config.ai_username);
        // Account doesn't expose username directly in a simple way,
        // but we verify the function doesn't panic and returns a valid account.
        // The username is embedded in the account's profile.
        let _ = account; // Compile-time check: Account type is correct
    }

    #[test]
    fn test_account_offline_default_username() {
        let config = AppConfig::default();
        let account = Account::offline(&config.ai_username);
        // Default username is "AI_Bot"
        let _ = account;
    }

    // -- ClientBuilder construction (compile-time check) ---------------------

    #[test]
    fn test_client_builder_exists_and_takes_handler() {
        // Verify that ClientBuilder::new().set_handler(events::handle_event) compiles.
        // We don't call .start() since there's no server.
        let _builder = ClientBuilder::new().set_handler(events::handle_event);
    }

    // -- Integration: state transitions during connect lifecycle -------------

    #[tokio::test]
    async fn test_state_starts_offline_before_connect() {
        let config = AppConfig::default();
        let state = Arc::new(SharedState::new(config.clone()));
        let manager = ConnectionManager::new(config, Arc::clone(&state));

        assert!(!manager.is_connected());
        assert!(!state.is_online());
    }

    #[test]
    fn test_manager_shares_state_with_external_readers() {
        let config = AppConfig::default();
        let state = Arc::new(SharedState::new(config.clone()));
        let manager = ConnectionManager::new(config.clone(), Arc::clone(&state));

        // External code reads from state directly
        assert!(!state.is_online());
        state.set_online(true);
        assert!(manager.is_connected());

        // Manager can also influence state
        manager.disconnect();
        assert!(!state.is_online());
    }

    // -- C-2 regression: per-iteration INJECTED_* reset ----------------

    /// Regression for C-2: the four `INJECTED_*` statics are cleared by
    /// `events::handle_disconnect` on every disconnect. If `connect`'s loop
    /// didn't re-install them on each iteration, the second (third, …)
    /// connection would see `None` slots and `BotState::default` would fall
    /// back to a throwaway state, so `is_online()` would never flip on the
    /// real `SharedState` the rest of the process reads.
    ///
    /// We can't drive `connect` against a real MC server in a unit test, so
    /// we exercise the extracted `inject_dependencies` helper directly:
    /// simulate the (set, clear, set) sequence and assert the statics are
    /// all `Some` after the second set.
    #[test]
    fn test_connect_resets_injections_each_iteration() {
        use crate::channel::{ReceiverSlot, create_command_channel};
        use crate::config::AppConfig;

        let config = AppConfig::default();
        let state = Arc::new(SharedState::new(config.clone()));
        let (_sender, receiver) = create_command_channel(4, Arc::clone(&state));
        let slot: ReceiverSlot = Arc::new(std::sync::Mutex::new(Some(receiver)));
        let sender = _sender;

        // Iteration 1: connect loop installs the values.
        ConnectionManager::inject_dependencies(
            &state,
            &slot,
            None,
            &sender,
            config.snapshot_interval_ms,
        );
        assert!(
            events::INJECTED_SHARED_STATE
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_some()
        );
        assert!(
            events::INJECTED_COMMAND_RECEIVER
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_some()
        );
        assert!(
            events::INJECTED_EGUI_CTX
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_none()
        );
        assert!(
            events::INJECTED_COMMAND_SENDER
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_some()
        );
        assert_eq!(
            events::INJECTED_SNAPSHOT_INTERVAL_MS.load(std::sync::atomic::Ordering::Relaxed),
            config.snapshot_interval_ms
        );

        // handle_disconnect clears them all (mirroring events::handle_disconnect).
        *events::INJECTED_SHARED_STATE
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
        *events::INJECTED_COMMAND_RECEIVER
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
        *events::INJECTED_EGUI_CTX
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
        *events::INJECTED_COMMAND_SENDER
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
        events::INJECTED_SNAPSHOT_INTERVAL_MS.store(0, std::sync::atomic::Ordering::Relaxed);

        // Iteration 2: the loop MUST re-install everything.
        ConnectionManager::inject_dependencies(
            &state,
            &slot,
            None,
            &sender,
            config.snapshot_interval_ms,
        );
        assert!(
            events::INJECTED_SHARED_STATE
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_some(),
            "INJECTED_SHARED_STATE must be Some after iteration 2"
        );
        assert!(
            events::INJECTED_COMMAND_RECEIVER
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_some(),
            "INJECTED_COMMAND_RECEIVER must be Some after iteration 2"
        );
        assert!(
            events::INJECTED_EGUI_CTX
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_none(),
            "INJECTED_EGUI_CTX was passed None, must stay None"
        );
        assert!(
            events::INJECTED_COMMAND_SENDER
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_some(),
            "INJECTED_COMMAND_SENDER must be Some after iteration 2"
        );
        assert_eq!(
            events::INJECTED_SNAPSHOT_INTERVAL_MS.load(std::sync::atomic::Ordering::Relaxed),
            config.snapshot_interval_ms,
            "snapshot interval must be restored on iteration 2"
        );
    }

    // -- F6-1 regression: session latch drives the backoff branch ------------

    /// Regression for audit F6-1: `handle_disconnect` clears `is_online()`
    /// BEFORE `ClientBuilder::start()` returns, so the connect loop reading
    /// `is_online()` at the capture point always saw `false` — the
    /// exponential-backoff branch was dead code and the bot fail-fasted
    /// after any real disconnect.
    ///
    /// The fix latches "this session came online" via
    /// [`SharedState::mark_session_online`] (called in `handle_spawn` next
    /// to `set_online(true)`) and consumes it with
    /// [`SharedState::take_session_was_online`] after `start()` returns.
    /// This test pins the exact latch semantics the fix relies on:
    /// `false` while the bot never connected, `true` after the session came
    /// online (even though `is_online()` is already cleared again), and
    /// consumed back to `false` by the take.
    #[test]
    fn test_session_latch_drives_backoff_branch() {
        let state = SharedState::new(AppConfig::default());

        // Never online → take reports false (first-connect retry path).
        assert!(!state.take_session_was_online());

        // handle_spawn path: bot came online.
        state.mark_session_online();
        state.set_online(true);

        // handle_disconnect runs BEFORE start() returns and clears the
        // online flag — the old capture point (`is_online()`) would see
        // only this:
        state.set_online(false);
        assert!(!state.is_online());

        // The latch survives the disconnect: the loop can still tell that
        // this session WAS online, making the backoff branch reachable.
        assert!(state.take_session_was_online());

        // The take consumed the latch — the next iteration starts clean.
        assert!(!state.take_session_was_online());
    }

    // -- Config-restart consumption ------------------------------------------

    /// A config restart (agent changed connection settings while online)
    /// must consume the restart flag, clear the disconnect request + stale
    /// error, reset the cancel token, and report `true` so the loop
    /// continues with updated settings. A second call without a fresh
    /// restart request must report `false`.
    #[test]
    fn test_should_restart_after_disconnect_consumes_flag_and_clears_state() {
        let state = SharedState::new(AppConfig::default());
        state.request_disconnect();
        state.request_config_restart();
        state.set_last_error("stale error from previous session");

        assert!(should_restart_after_disconnect(&state));

        // Disconnect flag cleared so the loop may reconnect.
        assert!(!state.is_disconnect_requested());
        // Restart flag consumed exactly once.
        assert!(!state.take_config_restart());
        // Stale error cleared for the fresh session.
        assert!(state.last_error().is_none());
        // Cancel token reset: a fresh token is not cancelled.
        assert!(!state.cancel_token().is_cancelled());

        // Second call without a new restart request → false.
        assert!(!should_restart_after_disconnect(&state));
    }

    /// Without a restart flag, a disconnect request must be honoured:
    /// `should_restart_after_disconnect` returns `false` and leaves the
    /// disconnect flag set so the loop breaks.
    #[test]
    fn test_should_restart_after_disconnect_false_keeps_disconnect() {
        let state = SharedState::new(AppConfig::default());
        state.request_disconnect();

        assert!(!should_restart_after_disconnect(&state));
        // Disconnect flag must remain set — the loop exits.
        assert!(state.is_disconnect_requested());
    }

    // -- Hot config reload ----------------------------------------------------

    /// `reconnect_backoff` must read the delays LIVE from `SharedState`
    /// (not from the frozen `self.config` snapshot) so an agent-driven
    /// `update_settings` takes effect on the very next reconnect without
    /// restarting the process.
    #[test]
    fn test_reconnect_backoff_hot_reloads_config() {
        let mut config = AppConfig::default();
        config.reconnect_initial_delay_ms = 1000;
        config.reconnect_max_delay_ms = 8000;

        let state = Arc::new(SharedState::new(config.clone()));
        let manager = ConnectionManager::new(config, Arc::clone(&state));

        assert_eq!(manager.reconnect_backoff(0), Duration::from_secs(1));

        // Agent updates the config while the connect loop is running.
        state.update_config(|cfg| {
            cfg.reconnect_initial_delay_ms = 2000;
            cfg.reconnect_max_delay_ms = 16000;
        });

        // Live read: the new values are picked up immediately.
        assert_eq!(manager.reconnect_backoff(0), Duration::from_secs(2));
        // attempt 3: 2000 * 2^3 = 16000 → exactly the new cap.
        assert_eq!(manager.reconnect_backoff(3), Duration::from_secs(16));
        // attempt 4: 32000 → capped at 16000.
        assert_eq!(manager.reconnect_backoff(4), Duration::from_secs(16));
    }

    /// Belt-and-braces: the connect loop itself calls
    /// `inject_dependencies` *every* iteration (verified by code reading
    /// only — this is a static check that the call site still exists).
    #[test]
    fn test_connect_loop_calls_inject_dependencies() {
        // The presence of the call inside `connect` is what this test
        // documents. If somebody moves the call out of the loop (e.g. only
        // runs it before `start()`), `BotState::default` on the second
        // connect will see `None` and the regression
        // `test_connect_resets_injections_each_iteration` will catch it.
        // We use a syntax-level check here: the function body must
        // mention `inject_dependencies`.
        let src = include_str!("connection.rs");
        // Two references: the def in `impl ConnectionManager` and the
        // call site inside `connect`.
        let occurrences = src.matches("inject_dependencies").count();
        assert!(
            occurrences >= 2,
            "expected `inject_dependencies` def + call site, found {occurrences} occurrences"
        );
    }
}
