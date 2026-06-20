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
use crate::channel::ReceiverSlot;
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

    /// Run the connection loop.
    ///
    /// Connects, runs until the bot is disconnected, then decides what to
    /// do next based on whether the bot ever came online:
    ///
    /// - **User-initiated disconnect** (`is_disconnect_requested()`): stop
    ///   immediately without writing an error.
    /// - **Connection never succeeded** (`is_online()` was `false` for the
    ///   entire `start()` call): capture a descriptive error into
    ///   [`SharedState::set_last_error`] and stop — the user must click
    ///   Connect again to retry.
    /// - **Was online, then disconnected** (a proper "reconnect" scenario):
    ///   clear `last_error` and retry with exponential backoff. The backoff
    ///   sleep is cancelable via the
    ///   [`CancellationToken`](tokio_util::sync::CancellationToken) so a
    ///   Disconnect click interrupts it immediately.
    ///
    /// Spawn this as a background task via [`tokio::spawn`].
    ///
    /// # Parameters
    /// - `command_receiver`: shared slot holding the command receiver, wrapped
    ///   in `Arc<Mutex<Option<_>>>` so the azalea event handler can lease it
    ///   on `Event::Spawn` (and return it on disconnect). Shared across
    ///   reconnection attempts.
    /// - `egui_ctx`: optional egui context for triggering UI repaints.
    pub async fn connect(
        &self,
        command_receiver: ReceiverSlot,
        egui_ctx: Option<egui::Context>,
    ) -> eyre::Result<()> {
        // Inject dependencies so BotState::default() picks them up when
        // azalea initializes the state internally via Default.
        let _ = events::INJECTED_SHARED_STATE.set(Arc::clone(&self.state));
        let _ = events::INJECTED_COMMAND_RECEIVER.set(Arc::clone(&command_receiver));
        let _ = events::INJECTED_EGUI_CTX.set(egui_ctx.clone());
        let _ = events::INJECTED_SNAPSHOT_INTERVAL_MS.set(self.config.snapshot_interval_ms);

        // Clear any stale disconnect request and error from a previous
        // session, and install a fresh cancellation token so a prior
        // session's cancel doesn't immediately trip our backoff sleep.
        self.state.clear_disconnect_request();
        self.state.clear_last_error();
        self.state.reset_cancel_token();

        let mut attempt: u32 = 0;

        loop {
            // If the user clicked Disconnect before we even started, stop.
            if self.state.is_disconnect_requested() {
                info!("disconnect requested — stopping connection loop");
                break;
            }

            let account = Account::offline(&self.config.ai_username);
            let address = format!("{}:{}", self.config.mc_address, self.config.mc_port);

            info!(
                "Connecting to {} as {} (attempt {})...",
                address,
                self.config.ai_username,
                attempt + 1
            );

            // start() blocks until the client disconnects or the connection fails.
            // BotState is created internally by azalea via Default — the injected
            // statics above ensure the correct SharedState and command receiver are used.
            let exit = ClientBuilder::new()
                .set_handler(events::handle_event)
                .start(account, address.clone())
                .await;

            // Was the bot online before this disconnect? The event handler
            // sets `is_online()` to true on `Event::Spawn`, so this is true
            // iff the bot successfully connected at some point during the
            // `start()` call. Capture it before we clear the flag below.
            let was_online = self.state.is_online();

            // Disconnected — ensure the online flag is cleared.
            self.state.set_online(false);

            // If the user requested disconnect, don't treat it as a failure.
            if self.state.is_disconnect_requested() {
                info!("disconnect requested — stopping reconnect loop");
                break;
            }

            if !was_online {
                // Connection never succeeded — fail fast. Capture a
                // descriptive error (including the AppExit details) so the
                // UI can display it, and stop retrying. The user must click
                // Connect again to attempt reconnection.
                let exit_desc = match &exit {
                    AppExit::Success => "success".to_string(),
                    AppExit::Error(code) => format!("error(code={code})"),
                };
                let msg = format!("Connection failed: {address} ({exit_desc})");
                warn!(%address, %exit_desc, "connection failed — stopping retry loop");
                self.state.set_last_error(msg);
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
        let initial_ms = self.config.reconnect_initial_delay_ms;
        let max_ms = self.config.reconnect_max_delay_ms;
        let delay_ms = initial_ms.saturating_mul(2u64.saturating_pow(attempt));
        Duration::from_millis(delay_ms.min(max_ms))
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
}
