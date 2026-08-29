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
//! [`ConnectionManager`] setup,
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
//!   clears the flag itself via `ClearGuard` when it exits.

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
///
/// `pub` (not `pub(crate)`) because the binary entry point (`src/main.rs`)
/// is a separate crate and needs it to bound its joins in headless mode.
pub fn join_with_timeout(handle: JoinHandle<()>, timeout: Duration) -> Result<(), Duration> {
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
/// 1. Installs a `ClearGuard` so the connecting flag is cleared even on
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

// ── Headless supervisor ─────────────────────────────────────────────────────

/// Next action the headless supervisor should take, decided from the current
/// [`SharedState`] flags.
///
/// Ordering matters:
/// 1. A cancelled shutdown token wins — the process is exiting.
/// 2. A pending config restart is consumed next (the flag is taken so a
///    second call in the same decision round cannot double-consume it).
/// 3. Otherwise spawn a connection when the bot is idle (offline, not
///    connecting, and no explicit disconnect was requested — `disconnect_bot`
///    must not be undone by an automatic respawn).
/// 4. Everything else is `WaitMore`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeadlessAction {
    /// Spawn the bot connection (bot idle, no pending restart or shutdown).
    SpawnConnect,
    /// No action this round — poll again.
    WaitMore,
    /// A config restart was requested; disconnect flags were cleared, loop to
    /// respawn with the fresh config.
    RestartWithNewConfig,
    /// The shutdown token is cancelled — exit the supervisor loop.
    Shutdown,
}

/// Decide the headless supervisor's next action from `state`.
pub fn headless_next_action(state: &SharedState) -> HeadlessAction {
    if state.shutdown_token().is_cancelled() {
        return HeadlessAction::Shutdown;
    }
    if state.take_config_restart() {
        state.clear_disconnect_request();
        return HeadlessAction::RestartWithNewConfig;
    }
    if !state.is_online() && !state.is_connecting() && !state.is_disconnect_requested() {
        return HeadlessAction::SpawnConnect;
    }
    HeadlessAction::WaitMore
}

/// Outcome of one [`inner_wait_step`] poll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InnerWaitOutcome {
    /// The shutdown token is cancelled — abort the bot and exit the
    /// supervisor.
    Shutdown,
    /// The bot thread is gone (finished, or no handle is stored) — the
    /// outer loop may proceed to the quiet-wait.
    ThreadGone,
    /// The bot thread is alive and healthy — sleep and poll again.
    WaitMore,
}

/// Decide one step of the supervisor's inner wait loop (the loop that
/// polls a LIVE bot thread handle while staying responsive to shutdown).
///
/// While a bot thread is alive the supervisor does NOT consume
/// [`SharedState::take_config_restart`]: an online config restart is
/// handled in-place by the connect loop inside the bot thread (its
/// checkpoint consumes the flag, clears the disconnect request, resets
/// the cancel token and reconnects — the thread never exits, so the
/// supervisor keeps polling the same handle). Consuming the flag here too
/// would race the connect loop and could spawn a SECOND bot thread — two
/// azalea sessions with the same username → server kick loop (M-10,
/// single-ownership rule: the connect loop owns the restart flag while a
/// bot thread lives; the supervisor consumes it only when no thread
/// exists).
pub(crate) fn inner_wait_step(
    state: &SharedState,
    handle: Option<&std::thread::JoinHandle<()>>,
) -> InnerWaitOutcome {
    if state.shutdown_token().is_cancelled() {
        return InnerWaitOutcome::Shutdown;
    }
    match handle {
        Some(h) if h.is_finished() => InnerWaitOutcome::ThreadGone,
        Some(_) => InnerWaitOutcome::WaitMore,
        // No handle stored — nothing to wait on; treat as "gone" so the
        // outer loop reaches the quiet-wait, which DOES own the restart
        // flag (no bot thread exists).
        None => InnerWaitOutcome::ThreadGone,
    }
}

/// Outcome of one [`quiet_wait_step`] poll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QuietWaitOutcome {
    /// The shutdown token is cancelled — exit the supervisor.
    Shutdown,
    /// No bot thread appeared and the config-restart flag was consumed —
    /// the outer loop respawns with the fresh config.
    Restart,
    /// A bot thread appeared while we waited (e.g. the agent called
    /// `connect_bot`) — step back to the handle-polling wait WITHOUT
    /// consuming the restart flag.
    ThreadAppeared,
    /// Nothing happened — keep waiting quietly.
    KeepWaiting,
}

/// Decide one step of the supervisor's quiet-wait loop (the loop that runs
/// when NO bot thread exists).
///
/// This is the ONLY place the supervisor consumes the config-restart flag —
/// but it must re-check for a live bot thread first (2026-08-29 review): a
/// thread can appear while the wait is in progress (agent calls
/// `connect_bot` → `spawn_bot_connection` → `store_bot_thread_handle`), and
/// the step-2 wait had already handed the handle off, so polling only the
/// shutdown token here would miss it. If a live thread exists, the connect
/// loop inside it owns the restart flag (M-10 single-ownership rule) — the
/// old code consumed it anyway, silently killing an online config restart
/// and leaving the bot offline until the agent acted again.
pub(crate) fn quiet_wait_step(state: &SharedState) -> QuietWaitOutcome {
    if state.shutdown_token().is_cancelled() {
        return QuietWaitOutcome::Shutdown;
    }
    if state.bot_thread_running() {
        return QuietWaitOutcome::ThreadAppeared;
    }
    if state.take_config_restart() {
        state.clear_disconnect_request();
        return QuietWaitOutcome::Restart;
    }
    QuietWaitOutcome::KeepWaiting
}

/// Initial backoff before retrying a FAILED bot-thread spawn (report M-8).
pub(crate) const SPAWN_RETRY_INITIAL: Duration = Duration::from_secs(5);
/// Cap for the failed-spawn retry backoff (report M-8).
pub(crate) const SPAWN_RETRY_MAX: Duration = Duration::from_secs(60);

/// Next backoff after another failed bot-thread spawn (report M-8):
/// exponential, capped at [SPAWN_RETRY_MAX]. Pure so the retry schedule
/// is unit-testable without spawning threads.
pub(crate) fn next_spawn_backoff(current: Duration) -> Duration {
    current.saturating_mul(2).min(SPAWN_RETRY_MAX)
}

/// Headless-mode supervisor loop (runs on the "headless-supervisor" OS
/// thread).
///
/// Responsibilities:
/// - Auto-connect the bot on startup.
/// - Re-spawn the connection after agent-driven config changes
///   (SharedState::request_config_restart) — e.g. the update_settings
///   MCP tool changing mc_address while the bot is offline.
/// - Exit when the shutdown token is cancelled (the MCP thread triggers it
///   once the stdio transport closes).
///
/// The supervisor does NOT hot-respawn in a loop: while a bot thread is
/// running it polls the thread plus the shutdown flag (rather than
/// blocking on a bare join()), so the moment the shutdown token fires it
/// can abort an in-flight azalea connect attempt via
/// SharedState::request_disconnect — otherwise a dead Minecraft server
/// would keep the process alive for the duration of azalea's internal TCP
/// retries. Online config restarts are NOT consumed here: while a bot
/// thread lives, the connect loop inside it owns the restart flag and
/// reconnects in-place (single-ownership rule, M-10 — the thread never
/// exits, so the supervisor keeps polling the same handle). If the bot
/// thread exits without a config restart (fail-fast after the bounded
/// first-connect retries, or an explicit disconnect_bot), the supervisor
/// waits quietly for a restart or a shutdown instead of re-spawning.
///
/// Report M-8: an OS-THREAD CREATION FAILURE (spawn_bot_connection error —
/// e.g. resource exhaustion) is NOT a bot-thread exit, so it must not fall
/// into the quiet wait — nothing would ever respawn and the process would
/// sit offline until shutdown. The supervisor retries the spawn on a
/// shutdown-responsive capped exponential backoff instead.
pub fn headless_supervisor(
    state: Arc<SharedState>,
    command_receiver: ReceiverSlot,
    command_sender: BotCommandSender,
) {
    let mut spawn_backoff = SPAWN_RETRY_INITIAL;
    loop {
        // ── 1. Decide the next action from the current flags ──────────────
        let mut spawn_failed = false;
        match headless_next_action(&state) {
            HeadlessAction::Shutdown => break,
            HeadlessAction::RestartWithNewConfig => continue,
            HeadlessAction::SpawnConnect => {
                if state.try_begin_connecting() {
                    match spawn_bot_connection(
                        Arc::clone(&state),
                        command_receiver.clone(),
                        command_sender.clone(),
                        None,
                    ) {
                        Ok(()) => {
                            // A successful spawn resets the failure backoff.
                            spawn_backoff = SPAWN_RETRY_INITIAL;
                        }
                        Err(e) => {
                            tracing::error!(error = %e, "headless: failed to spawn bot connection");
                            state.set_last_error(format!("failed to spawn bot connection: {e}"));
                            state.clear_connecting();
                            spawn_failed = true;
                        }
                    }
                }
            }
            HeadlessAction::WaitMore => {}
        }

        // ── 1b. M-8: a spawn failure must NOT fall into the thread wait /
        //        quiet wait below — with no bot thread ever created there is
        //        nothing to join and (crucially) no config-restart consumer,
        //        so the supervisor would sit offline forever. Back off with
        //        a capped exponential delay (staying responsive to shutdown)
        //        and retry the spawn on the next round.
        if spawn_failed {
            let deadline = std::time::Instant::now() + spawn_backoff;
            loop {
                if state.shutdown_token().is_cancelled() {
                    break;
                }
                if std::time::Instant::now() >= deadline {
                    break;
                }
                std::thread::sleep(Duration::from_millis(250));
            }
            spawn_backoff = next_spawn_backoff(spawn_backoff);
            if state.shutdown_token().is_cancelled() {
                break;
            }
            continue;
        }

        // ── 2. Wait for the bot thread while staying responsive to
        //       shutdown ───────────────────────────────────────────────────
        let mut handle = state.take_bot_thread_handle();
        let mut break_outer = false;
        loop {
            match inner_wait_step(&state, handle.as_ref()) {
                InnerWaitOutcome::Shutdown => {
                    // Abort the bot's in-flight connect attempt
                    // (request_disconnect cancels the token the connect
                    // loop's select! is waiting on) and join with a bound.
                    state.request_disconnect();
                    if let Some(handle) = handle.take() {
                        let _ = join_with_timeout(handle, Duration::from_secs(3));
                    }
                    break_outer = true;
                    break;
                }
                InnerWaitOutcome::ThreadGone => break,
                InnerWaitOutcome::WaitMore => {
                    // Note (M-10): the config-restart flag is deliberately
                    // NOT consumed here. While a bot thread lives, the
                    // connect loop inside it owns the flag — it consumes
                    // the restart, clears the disconnect request, resets
                    // the cancel token and reconnects in-place, so this
                    // thread never exits and the supervisor keeps polling
                    // the same handle. Consuming the flag here too would
                    // let the old loop fall into backoff while the
                    // supervisor spawns a second bot thread.
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
        if break_outer {
            break;
        }

        // ── 3. Thread gone. Wait quietly for a restart or shutdown — an
        //       explicit disconnect_bot must stay effective and a
        //       fail-fast must not spin the connection loop. Only here,
        //       with NO bot thread alive, does the supervisor consume the
        //       config-restart flag (single-ownership rule, M-10); the
        //       outer loop then respawns via headless_next_action. The
        //       step re-checks for a live thread first: `connect_bot` may
        //       have spawned one while we waited (QuietWaitOutcome::
        //       ThreadAppeared) — the flag stays untouched for that
        //       thread's connect loop.
        loop {
            match quiet_wait_step(&state) {
                QuietWaitOutcome::Shutdown => {
                    break_outer = true;
                    break;
                }
                QuietWaitOutcome::Restart | QuietWaitOutcome::ThreadAppeared => break,
                QuietWaitOutcome::KeepWaiting => {
                    std::thread::sleep(Duration::from_millis(250));
                }
            }
        }
        if break_outer {
            break;
        }
    }

    // Supervisor exited: tell the bot to stop and bound the final join.
    state.request_disconnect();
    if let Some(handle) = state.take_bot_thread_handle() {
        let _ = join_with_timeout(handle, Duration::from_secs(3));
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::{ReceiverSlot, create_command_channel};
    use crate::config::AppConfig;

    /// M-8: the failed-spawn retry schedule doubles up to the cap and then
    /// stays there — a persistently failing thread creation never wedges
    /// the supervisor (it keeps retrying) and never hot-spins either.
    #[test]
    fn test_next_spawn_backoff_doubles_to_cap() {
        assert_eq!(SPAWN_RETRY_INITIAL, Duration::from_secs(5));
        assert_eq!(
            next_spawn_backoff(SPAWN_RETRY_INITIAL),
            Duration::from_secs(10)
        );
        assert_eq!(
            next_spawn_backoff(Duration::from_secs(10)),
            Duration::from_secs(20)
        );
        assert_eq!(
            next_spawn_backoff(Duration::from_secs(20)),
            Duration::from_secs(40)
        );
        assert_eq!(next_spawn_backoff(Duration::from_secs(40)), SPAWN_RETRY_MAX);
        assert_eq!(next_spawn_backoff(SPAWN_RETRY_MAX), SPAWN_RETRY_MAX);
    }

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

    // ── headless_next_action state machine ────────────────────────────

    /// Idle bot (offline, not connecting, no pending disconnect/restart,
    /// shutdown not triggered) → spawn a connection.
    #[test]
    fn test_next_action_idle_bot_spawns() {
        let state = SharedState::new(AppConfig::default());
        assert_eq!(headless_next_action(&state), HeadlessAction::SpawnConnect);
    }

    /// A cancelled shutdown token wins over everything else.
    #[test]
    fn test_next_action_shutdown_wins() {
        let state = SharedState::new(AppConfig::default());
        state.trigger_shutdown();
        assert_eq!(headless_next_action(&state), HeadlessAction::Shutdown);
    }

    /// A pending config restart is consumed and reported, and a second call
    /// in the same round no longer sees it (the flag was taken).
    #[test]
    fn test_next_action_config_restart_consumed_once() {
        let state = SharedState::new(AppConfig::default());
        state.request_config_restart();
        assert_eq!(
            headless_next_action(&state),
            HeadlessAction::RestartWithNewConfig
        );
        // Flag consumed → the idle bot falls through to spawn.
        assert_eq!(headless_next_action(&state), HeadlessAction::SpawnConnect);
    }

    /// Restart also clears any pending disconnect request so the respawn
    /// isn't immediately aborted.
    #[test]
    fn test_next_action_restart_clears_disconnect() {
        let state = SharedState::new(AppConfig::default());
        state.request_disconnect();
        state.request_config_restart();
        assert_eq!(
            headless_next_action(&state),
            HeadlessAction::RestartWithNewConfig
        );
        assert!(
            !state.is_disconnect_requested(),
            "restart consumption must clear the disconnect request"
        );
    }

    /// A bot currently connecting must NOT get a duplicate spawn.
    #[test]
    fn test_next_action_connecting_waits() {
        let state = SharedState::new(AppConfig::default());
        assert!(state.try_begin_connecting());
        assert_eq!(headless_next_action(&state), HeadlessAction::WaitMore);
    }

    /// An explicit disconnect request (the `disconnect_bot` tool) suppresses
    /// auto-respawn — the supervisor must wait, not reconnect immediately.
    #[test]
    fn test_next_action_disconnect_requested_waits() {
        let state = SharedState::new(AppConfig::default());
        state.request_disconnect();
        assert_eq!(headless_next_action(&state), HeadlessAction::WaitMore);
    }

    /// A bot already online waits (no re-spawn while the thread is alive).
    #[test]
    fn test_next_action_online_waits() {
        let state = SharedState::new(AppConfig::default());
        state.set_online(true);
        assert_eq!(headless_next_action(&state), HeadlessAction::WaitMore);
    }

    // ── M-10 regression: single ownership of the restart flag ───────────

    /// Regression for audit M-10: while a bot thread is alive, the
    /// supervisor's inner wait loop must NOT consume the config-restart
    /// flag.
    ///
    /// An online config restart (`update_settings` while the bot is
    /// connected) is handled IN-PLACE by the connect loop inside the bot
    /// thread: its checkpoint consumes the flag, clears the disconnect
    /// request, resets the cancel token and reconnects — the thread never
    /// exits, so the supervisor keeps polling the SAME handle. If the
    /// supervisor also consumed the flag (and cleared the disconnect
    /// request), the old loop would fall into its backoff branch and
    /// reconnect on its own thread while the supervisor spawns a SECOND
    /// bot thread → two azalea sessions with the same username → server
    /// kick loop.
    ///
    /// This test drives the extracted [`inner_wait_step`]: with a live,
    /// non-finished handle and the restart flag set, the step must WAIT —
    /// the flag must remain set for the connect loop to consume.
    #[test]
    fn test_supervisor_does_not_consume_restart_while_thread_alive() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let state = SharedState::new(AppConfig::default());

        // A live, non-finished bot thread handle.
        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = Arc::clone(&stop);
        let handle = std::thread::spawn(move || {
            while !stop_clone.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(5));
            }
        });

        // Agent changed connection settings while the bot was online.
        state.request_config_restart();

        // The inner-loop step must WAIT — the flag stays set for the
        // connect loop (inside the bot thread) to consume.
        assert_eq!(
            inner_wait_step(&state, Some(&handle)),
            InnerWaitOutcome::WaitMore
        );
        assert!(
            state.take_config_restart(),
            "restart flag must stay set while a bot thread lives — the connect loop owns it"
        );

        // A finished thread is reported gone so the outer loop can proceed.
        stop.store(true, Ordering::Relaxed);
        // Bounded wait for the thread to exit without consuming the handle
        // (join() moves it; we still need `is_finished()` below).
        for _ in 0..200 {
            if handle.is_finished() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(handle.is_finished(), "parked thread should exit promptly");
        assert_eq!(
            inner_wait_step(&state, Some(&handle)),
            InnerWaitOutcome::ThreadGone
        );
        // No thread at all is also "gone" (nothing to wait on).
        assert_eq!(inner_wait_step(&state, None), InnerWaitOutcome::ThreadGone);
    }

    /// RED (2026-08-29 review): the quiet-wait consumes the config-restart
    /// flag "only when NO bot thread exists". A thread can appear while the
    /// wait is in progress (agent calls `connect_bot`), and with only the
    /// shutdown token + flag being polled the wait never noticed it — it
    /// consumed the flag an online `update_settings` had left for the
    /// connect loop, silently killing the restart. `quiet_wait_step` must
    /// re-check for a live thread BEFORE consuming the flag.
    #[test]
    fn test_quiet_wait_step_thread_appeared_keeps_restart_flag() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let state = SharedState::new(AppConfig::default());

        // A live, non-finished bot thread handle (stored, as
        // `spawn_bot_connection` would).
        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = Arc::clone(&stop);
        let handle = std::thread::spawn(move || {
            while !stop_clone.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(5));
            }
        });
        state.store_bot_thread_handle(handle);

        // Agent changed connection settings while the (new) bot was online.
        state.request_config_restart();

        assert_eq!(
            quiet_wait_step(&state),
            QuietWaitOutcome::ThreadAppeared,
            "a live thread must step the quiet-wait back to the handle-polling wait"
        );
        assert!(
            state.take_config_restart(),
            "restart flag must stay set for the connect loop (M-10 single ownership)"
        );

        // Once the thread finishes, the quiet-wait owns the flag again.
        stop.store(true, Ordering::Relaxed);
        let handle = state.take_bot_thread_handle().expect("handle stored");
        for _ in 0..200 {
            if handle.is_finished() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(handle.is_finished(), "parked thread should exit promptly");
        state.request_config_restart();
        assert_eq!(
            quiet_wait_step(&state),
            QuietWaitOutcome::Restart,
            "with no live thread the quiet-wait consumes the restart flag"
        );
        assert!(
            !state.take_config_restart(),
            "Restart outcome must have consumed the flag"
        );
    }

    /// No thread and no restart flag → keep waiting.
    #[test]
    fn test_quiet_wait_step_no_thread_no_flag_keeps_waiting() {
        let state = SharedState::new(AppConfig::default());
        assert_eq!(
            quiet_wait_step(&state),
            QuietWaitOutcome::KeepWaiting
        );
        assert!(
            !state.take_config_restart(),
            "KeepWaiting must not consume the restart flag"
        );
    }

    /// Shutdown wins over everything else in the quiet-wait.
    #[test]
    fn test_quiet_wait_step_shutdown() {
        let state = SharedState::new(AppConfig::default());
        state.trigger_shutdown();
        assert_eq!(quiet_wait_step(&state), QuietWaitOutcome::Shutdown);
    }

    /// `bot_thread_running` probes without consuming the stored handle.
    #[test]
    fn test_bot_thread_running_three_states() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let state = SharedState::new(AppConfig::default());
        assert!(!state.bot_thread_running(), "no handle stored → not running");

        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = Arc::clone(&stop);
        let handle = std::thread::spawn(move || {
            while !stop_clone.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(5));
            }
        });
        state.store_bot_thread_handle(handle);
        assert!(state.bot_thread_running(), "live handle → running");

        stop.store(true, Ordering::Relaxed);
        let handle = state.take_bot_thread_handle().expect("handle still stored");
        for _ in 0..200 {
            if handle.is_finished() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(handle.is_finished());
        // The take() above removed the handle, so store it back to probe
        // the finished state through the accessor.
        state.store_bot_thread_handle(handle);
        assert!(
            !state.bot_thread_running(),
            "finished handle → not running"
        );
        let _ = state.take_bot_thread_handle();
    }
}
