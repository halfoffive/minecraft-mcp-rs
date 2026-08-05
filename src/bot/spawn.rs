//! Bot connection thread spawn helper.
//!
//! Extracted from `MinecraftApp::connect_bot` so that the headless
//! supervisor and the `connect_bot` MCP tool can spawn bot connections
//! without any UI involvement.
//!
//! The bot connection MUST run on a dedicated OS thread (not
//! `tokio::spawn`) because azalea's
//! [`ClientBuilder::start`](azalea::ClientBuilder::start) internally
//! creates a `LocalSet` which is `!Send`.
//!
//! # Responsibilities
//!
//! [`spawn_bot_connection`] owns the thread body (runtime creation,
//! [`ConnectionManager`](crate::bot::connection::ConnectionManager) setup,
//! error logging) and stores the resulting [`JoinHandle`] in
//! [`SharedState`] via
//! [`store_bot_thread_handle`](crate::state::SharedState::store_bot_thread_handle).
//! Callers are responsible for:
//!
//! - claiming the connecting flag via
//!   [`try_begin_connecting`](crate::state::SharedState::try_begin_connecting)
//!   **before** calling, and
//! - calling
//!   [`clear_connecting`](crate::state::SharedState::clear_connecting) if
//!   the spawn itself fails (the helper returns the `io::Error` without
//!   touching the flag on that path). On a successful spawn the thread
//!   clears the flag itself via [`ClearGuard`] when it exits.

use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use crate::bot::connection::ConnectionManager;
use crate::channel::{BotCommandSender, ReceiverSlot};
use crate::state::SharedState;

// ── Thread join helper ─────────────────────────────────────────────────────

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
pub(crate) fn join_with_timeout(handle: JoinHandle<()>, timeout: Duration) -> Result<(), Duration> {
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

// ── Connecting-flag guard ──────────────────────────────────────────────────

/// RAII guard that calls [`SharedState::clear_connecting`] on drop.
///
/// Wrapping the bot connection thread body with this guard ensures the
/// `bot_connecting` flag is always cleared — even if a panic unwinds
/// through the thread (e.g. `Runtime::new().expect()`). Without it, a
/// panic in the connection thread would leave `bot_connecting` permanently
/// `true`, and `try_begin_connecting` would reject every subsequent
/// Connect attempt until the process restarts.
///
/// # Safety for panics
///
/// The guard *only* touches an `AtomicBool` in `drop`, which is safe
/// during unwinding. If the thread panics, `clear_connecting` still
/// executes (unwinding calls `Drop`), so the user can retry Connect
/// without restarting the application.
pub(crate) struct ClearGuard<'a>(pub(crate) &'a SharedState);

impl Drop for ClearGuard<'_> {
    fn drop(&mut self) {
        self.0.clear_connecting();
    }
}

// ── Spawn helper ───────────────────────────────────────────────────────────

/// Spawn the bot connection on a dedicated OS thread named
/// `"bot-connection"`.
///
/// The thread body (preserved verbatim from the original
/// `MinecraftApp::connect_bot` implementation):
///
/// 1. Installs a [`ClearGuard`] so the connecting flag is cleared even on
///    panic.
/// 2. Creates a tokio [`Runtime`](tokio::runtime::Runtime); on failure
///    logs the error and returns (the guard clears the flag).
/// 3. Reads a **fresh** config clone from `state` inside the thread so
///    each spawn honours the latest settings.
/// 4. Runs [`ConnectionManager::connect`] to completion, logging any
///    error.
///
/// On successful spawn the [`JoinHandle`] is stored via
/// [`SharedState::store_bot_thread_handle`] so the UI / headless
/// supervisor can join it later, and `Ok(())` is returned.
///
/// # Errors
///
/// Returns the `io::Error` from [`std::thread::Builder::spawn`] if the OS
/// thread could not be created. The caller must then log the error, record
/// it via [`SharedState::set_last_error`] and clear the connecting flag —
/// the helper does not modify the flag on this path.
pub fn spawn_bot_connection(
    state: Arc<SharedState>,
    command_receiver: ReceiverSlot,
    command_sender: BotCommandSender,
    egui_ctx: Option<egui::Context>,
) -> std::io::Result<()> {
    let state_for_thread = Arc::clone(&state);

    match std::thread::Builder::new()
        .name("bot-connection".into())
        .spawn(move || {
            // RAII guard: `ClearGuard` calls `state.clear_connecting()`
            // on drop, so the flag is cleared even if this thread panics
            // (e.g. during `Runtime::new()` below).  Without this, a
            // panic would leave `bot_connecting` permanently `true`,
            // and the user would be unable to Connect again without
            // restarting the whole application.
            let _clear_guard = ClearGuard(&state_for_thread);

            let rt = match tokio::runtime::Runtime::new() {
                Ok(rt) => rt,
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        "Failed to create tokio runtime for bot connection"
                    );
                    // _clear_guard drops → clear_connecting() runs
                    return;
                }
            };
            // Fresh config per spawn: read inside the thread so settings
            // changed between the spawn call and thread start are honoured.
            let config = state_for_thread.read_config().clone();
            let manager = ConnectionManager::new(config, Arc::clone(&state_for_thread));

            rt.block_on(async move {
                if let Err(e) = manager
                    .connect(command_receiver, egui_ctx, command_sender)
                    .await
                {
                    tracing::error!(error = %e, "bot connection task failed");
                }
            });

            tracing::info!("Bot connection thread exited");
        }) {
        Ok(handle) => {
            state.store_bot_thread_handle(handle);
            tracing::info!("Bot connection thread spawned");
            Ok(())
        }
        Err(e) => Err(e),
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::{ReceiverSlot, create_command_channel};
    use crate::config::AppConfig;

    /// Dropping [`ClearGuard`] clears the `bot_connecting` flag, so a
    /// panicked or finished connection thread never wedges future Connect
    /// attempts.
    #[test]
    fn test_clear_guard_clears_connecting_flag_on_drop() {
        let state = SharedState::new(AppConfig::default());
        assert!(state.try_begin_connecting());
        assert!(state.is_connecting());
        {
            let _guard = ClearGuard(&state);
            // Flag stays set while the guard lives.
            assert!(state.is_connecting());
        }
        // Guard dropped → clear_connecting() ran.
        assert!(!state.is_connecting());
    }

    /// [`spawn_bot_connection`] stores the thread handle in [`SharedState`]
    /// so the UI / supervisor can join it later via
    /// [`SharedState::take_bot_thread_handle`].
    ///
    /// The config points at the closed port `127.0.0.1:1`, so the spawned
    /// thread fail-fasts on its own after the bounded first-connect retries
    /// (~a few seconds). We deliberately do NOT join the handle here to keep
    /// the test fast; the thread clears the connecting flag via
    /// [`ClearGuard`] when it exits and is reclaimed at process exit.
    #[test]
    fn test_spawn_bot_connection_stores_handle() {
        let config = AppConfig {
            mc_address: "127.0.0.1".into(),
            mc_port: 1, // closed port → thread fail-fasts by itself
            ..Default::default()
        };

        let state = Arc::new(SharedState::new(config));
        let (sender, receiver) = create_command_channel(4, Arc::clone(&state));
        let slot: ReceiverSlot = Arc::new(std::sync::Mutex::new(Some(receiver)));

        // Mirror production usage: the caller claims the connecting flag
        // before spawning.
        assert!(state.try_begin_connecting());

        spawn_bot_connection(Arc::clone(&state), slot, sender, None)
            .expect("spawning the bot connection thread should succeed");

        assert!(
            state.take_bot_thread_handle().is_some(),
            "spawn_bot_connection must store the JoinHandle in SharedState"
        );
        // Second take returns None — the handle was moved out.
        assert!(state.take_bot_thread_handle().is_none());
    }

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
