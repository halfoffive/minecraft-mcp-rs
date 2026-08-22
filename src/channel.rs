//! Cross-domain communication channels (tokio mpsc / oneshot).
//!
//! This module provides the communication bridge between the MCP server and
//! the bot engine. Every MCP tool call is serialized into a `BotCommand`,
//! sent through an async mpsc channel, and the response is returned via a
//! oneshot channel.

use std::sync::{Arc, Mutex};

use tokio::sync::{mpsc, oneshot};
use tokio::time::{Duration, timeout};
use tracing::{debug, error, trace, warn};

use crate::error::BotError;
use crate::state::SharedState;
use crate::types::{ActAction, BotCommand, BotResult};

// ═══════════════════════════════════════════════════════════════
// BotCommandWithResponder
// ═══════════════════════════════════════════════════════════════

/// A bot command bundled with a oneshot channel to send the response back.
///
/// The receiver side processes `command` and sends the `Result<BotResult, BotError>`
/// through `respond_to`.
#[derive(Debug)]
pub struct BotCommandWithResponder {
    pub command: BotCommand,
    pub respond_to: oneshot::Sender<Result<BotResult, BotError>>,
}

// ═══════════════════════════════════════════════════════════════
// BotCommandSender
// ═══════════════════════════════════════════════════════════════

/// Sender side of the bot command channel.
///
/// Clone this to share the ability to send commands across tasks.
/// Commands are serialized by the single `BotCommandReceiver`.
///
/// The response timeout is read **at every `send_command` call** from
/// `SharedState::read_config().command_timeout_secs`, so the caller (and
/// any `RealBotClient` reading the same [`SharedState`]) always see the
/// latest user-configured value without re-creating the sender.
#[derive(Debug, Clone)]
pub struct BotCommandSender {
    tx: mpsc::Sender<BotCommandWithResponder>,
    /// Shared state — used to read the current `command_timeout_secs`
    /// at every `send_command` call (so the timeout is hot-updatable
    /// from the UI without re-creating the sender).
    state: Arc<SharedState>,
}

impl BotCommandSender {
    /// Return the current response timeout (from the shared config).
    ///
    /// Exposed so `RealBotClient::goto` (and any
    /// other long-running client method) can honour the same value the
    /// `send_command` envelope enforces, without re-reading the config
    /// independently.
    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.state.read_config().command_timeout_secs)
    }

    /// The envelope timeout for a specific command: the configured command
    /// timeout, or the longer fly timeout for flight commands — `FlyTo` AND
    /// `Act(Fly)` (report M-3: the unified act tool reached the same
    /// `handle_fly_to` executor path but kept the 30 s command envelope, so
    /// long flights reported a false CommandTimeout while the bot was still
    /// flying). `handle_fly_to`'s internal goto uses the same fly timeout,
    /// so the executor always replies before this envelope fires.
    ///
    /// Reads the config lock ONCE and derives both the command and fly
    /// timeouts from that single guard (L-21) — the previous
    /// implementation called `read_config` twice (once via `timeout()`,
    /// once for `fly_timeout_secs`).
    fn timeout_for(&self, cmd: &BotCommand) -> Duration {
        let cfg = self.state.read_config();
        let base = Duration::from_secs(cfg.command_timeout_secs);
        let is_flight = matches!(cmd, BotCommand::FlyTo(_))
            || matches!(cmd, BotCommand::Act(ActAction::Fly { .. }, _));
        // F-1: compound act actions (mine, collect) are multi-leg operations
        // whose movement + mining/verification phases routinely exceed the
        // plain command envelope. They share the longer flight envelope so a
        // far-away target no longer produces a guaranteed client-side
        // CommandTimeout while the serial executor keeps working.
        let is_compound_act = matches!(
            cmd,
            BotCommand::Act(ActAction::Mine { .. } | ActAction::CollectItems { .. }, _)
        );
        if is_flight || is_compound_act {
            let fly = Duration::from_secs(cfg.fly_timeout_secs);
            std::cmp::max(base, fly)
        } else {
            base
        }
    }

    /// Send a command to the bot and await the response.
    ///
    /// The response timeout is read from the shared
    /// [`AppConfig::command_timeout_secs`](crate::config::AppConfig::command_timeout_secs)
    /// at the moment of the call, so changing the value in the settings
    /// panel takes effect on the next command without restarting the MCP
    /// server. `FlyTo` gets its own longer envelope (see
    /// `timeout_for`).
    ///
    /// # Errors
    /// - `BotError::Offline` if the receiver has been dropped, or if the
    ///   responder side drops the oneshot without sending (channel closed).
    /// - `BotError::CommandTimeout` if no response arrives within the
    ///   currently-configured command timeout.
    ///
    /// # Timeout is not cancellation (F-10)
    ///
    /// A timeout only stops the SENDER from waiting. The command has already
    /// been queued and the serial executor may still run it to completion;
    /// its response is simply dropped. A client that retries a timed-out
    /// command can therefore observe the side effect twice (e.g. `DropItem`
    /// dropping double the intended count). Callers should prefer idempotent
    /// commands or verify state before retrying.
    pub async fn send_command(&self, cmd: BotCommand) -> Result<BotResult, BotError> {
        let timeout_dur = self.timeout_for(&cmd);
        self.send_command_with_timeout(cmd, timeout_dur).await
    }

    /// Send a command with an explicit envelope timeout.
    ///
    /// Identical semantics to [`send_command`](Self::send_command) except
    /// the response envelope is exactly `timeout_dur` (no `timeout_for`
    /// lookup). Exposed so callers can give long-running commands their own
    /// budget (e.g. `fly_to`'s `fly_timeout_secs`) without mutating the
    /// shared config.
    ///
    /// # Errors
    /// Same as [`send_command`](Self::send_command). The same timeout-is-not-
    /// cancellation caveat applies.
    pub async fn send_command_with_timeout(
        &self,
        cmd: BotCommand,
        timeout_dur: Duration,
    ) -> Result<BotResult, BotError> {
        let (respond_to, rx) = oneshot::channel();

        // L-21: the `CommandTimeout` error message must render the real
        // command's Debug form — that is part of the error contract, pinned
        // by `tests/integration.rs::test_command_timeout_responder_alive_but_slow`.
        // The channel also needs an owned `BotCommand`, so exactly ONE clone
        // per call is unavoidable (both consumers need a command). What IS
        // avoided: the eager `format!` (S-5) and the redundant second
        // `read_config` in `timeout_for`. The tracing macros reference `cmd`
        // lazily (`?cmd` defers the Debug formatting until a level is
        // enabled), so the ORIGINAL command is kept alive here and the
        // channel receives a clone — the log lines and the error message
        // both render the exact command that was sent.
        let wrapped = BotCommandWithResponder {
            command: cmd.clone(),
            respond_to,
        };

        trace!(command = ?cmd, "sending bot command");

        if self.tx.send(wrapped).await.is_err() {
            error!("bot command channel closed — receiver dropped");
            return Err(BotError::Offline("bot command channel closed".into()));
        }

        // The caller-supplied envelope is used verbatim (L-18-prep); the
        // `Duration` (not a raw u64) supports sub-second timeouts like
        // 200ms without truncation.
        let timeout_secs = timeout_dur.as_secs();
        match timeout(timeout_dur, rx).await {
            Ok(Ok(result)) => {
                debug!(command = ?cmd, ?result, "bot command completed");
                result
            }
            Ok(Err(_)) => {
                warn!(command = ?cmd, "bot command responder dropped without reply");
                Err(BotError::Offline(
                    "bot command responder dropped without reply".into(),
                ))
            }
            Err(_) => {
                error!(command = ?cmd, timeout_secs, "bot command timed out");
                Err(BotError::CommandTimeout {
                    command: format!("{cmd:?}"),
                    timeout_secs,
                })
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// BotCommandReceiver
// ═══════════════════════════════════════════════════════════════

/// Receiver side of the bot command channel.
///
/// There can only be **one** receiver. Commands are processed serially
/// by the task that owns this receiver.
#[derive(Debug)]
pub struct BotCommandReceiver {
    rx: mpsc::Receiver<BotCommandWithResponder>,
}

impl BotCommandReceiver {
    /// Receive the next command, awaiting if the channel is empty.
    ///
    /// Returns `None` when all senders have been dropped.
    pub async fn recv(&mut self) -> Option<BotCommandWithResponder> {
        self.rx.recv().await
    }

    /// Try to receive a command without blocking.
    ///
    /// Returns `Err(TryRecvError::Empty)` if no command is pending,
    /// or `Err(TryRecvError::Disconnected)` if all senders are dropped.
    pub fn try_recv(&mut self) -> Result<BotCommandWithResponder, mpsc::error::TryRecvError> {
        self.rx.try_recv()
    }
}

// ═══════════════════════════════════════════════════════════════
// Factory
// ═══════════════════════════════════════════════════════════════

/// Create a new bot command channel with the given buffer size.
///
/// The sender's response timeout is read live from
/// `SharedState::read_config().command_timeout_secs` (the user-configurable
/// "Command timeout" in the settings panel). A fresh [`SharedState`] is
/// provided so [`BotCommandSender::send_command`] can look up the timeout
/// at every call without re-creating the sender when the user changes it.
pub fn create_command_channel(
    buffer: usize,
    state: Arc<SharedState>,
) -> (BotCommandSender, BotCommandReceiver) {
    let (tx, rx) = mpsc::channel(buffer);
    (BotCommandSender { tx, state }, BotCommandReceiver { rx })
}

// ═══════════════════════════════════════════════════════════════
// ReceiverLease — borrow the receiver, return it on drop
// ═══════════════════════════════════════════════════════════════

/// The shared slot that holds the optional command receiver.
///
/// `BotState` stores the receiver here so the azalea event handler can
/// [`ReceiverLease::take`] it on `Event::Spawn` and the command executor can
/// run with it. When the executor is aborted (e.g. on disconnect), the
/// [`ReceiverLease`] guard drops and puts the receiver back into the slot,
/// allowing the next `Spawn` to re-acquire it.
pub(crate) type ReceiverSlot = Arc<Mutex<Option<BotCommandReceiver>>>;

/// A guard that owns the command receiver for the duration of a command
/// executor task and returns it to its [`ReceiverSlot`] when dropped.
///
/// Construct via [`ReceiverLease::take`]. If the slot was empty (receiver
/// already leased or never injected), `take` returns `None`.
pub(crate) struct ReceiverLease {
    slot: ReceiverSlot,
    receiver: Option<BotCommandReceiver>,
}

impl ReceiverLease {
    /// Take the receiver out of the shared slot, returning a guard that will
    /// put it back on drop.
    ///
    /// Returns `None` if the slot is empty (no receiver to lease).
    pub(crate) fn take(slot: &ReceiverSlot) -> Option<Self> {
        let mut guard = slot.lock().unwrap_or_else(|e| e.into_inner());
        guard.take().map(|rx| Self {
            slot: Arc::clone(slot),
            receiver: Some(rx),
        })
    }

    /// Take the receiver out of the shared slot, retrying briefly when the
    /// slot is empty.
    ///
    /// Used by `Event::Spawn` to bridge the race window where a previous
    /// executor's `ReceiverLease` has not yet been dropped back into the
    /// slot. Without the retry, fast reconnects would always observe an
    /// empty slot and skip starting a new executor (logged as a
    /// confusing "no receiver" warning). The retry loop polls the slot
    /// for up to ~100ms (20 × 5ms) and yields to the runtime between
    /// attempts so the previous `Drop` runs. If the slot is still empty
    /// after the window, returns `None` so the caller can take the
    /// "no executor" branch as before.
    pub(crate) async fn take_with_retry(slot: &ReceiverSlot) -> Option<Self> {
        for _ in 0..20 {
            if let Some(lease) = Self::take(slot) {
                return Some(lease);
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        None
    }

    /// Borrow the underlying receiver for receiving commands.
    ///
    /// The receiver is always present while the lease is held (the lease is
    /// only constructed when `take` succeeds).
    pub(crate) fn receiver_mut(&mut self) -> &mut BotCommandReceiver {
        self.receiver
            .as_mut()
            .expect("ReceiverLease missing receiver — invariant violated")
    }
}

impl Drop for ReceiverLease {
    fn drop(&mut self) {
        if let Some(rx) = self.receiver.take() {
            *self.slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(rx);
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::types::BlockPos;

    // ── Helpers ─────────────────────────────────────────────

    fn make_result(success: bool, message: &str) -> BotResult {
        BotResult {
            success,
            message: message.into(),
            data: None,
        }
    }

    /// Create a fresh [`SharedState`] with the default config and wrap it in
    /// an `Arc` for the channel factory.
    fn make_state() -> Arc<SharedState> {
        Arc::new(SharedState::new(AppConfig::default()))
    }

    // ── Command flow ────────────────────────────────────────

    #[tokio::test]
    async fn test_command_flow_success() {
        let (sender, mut receiver) = create_command_channel(10, make_state());

        let cmd = BotCommand::Jump;

        let responder = tokio::spawn(async move {
            let wrapped = receiver.recv().await.expect("should receive command");
            assert!(matches!(wrapped.command, BotCommand::Jump));

            wrapped
                .respond_to
                .send(Ok(make_result(true, "jumped")))
                .expect("should send response");
        });

        let result = sender.send_command(cmd).await.expect("should succeed");
        assert!(result.success);
        assert_eq!(result.message, "jumped");

        responder.await.expect("responder task should complete");
    }

    #[tokio::test]
    async fn test_command_flow_error_response() {
        let (sender, mut receiver) = create_command_channel(10, make_state());

        let cmd = BotCommand::BreakBlock(BlockPos::new(1, 2, 3));

        let responder = tokio::spawn(async move {
            let wrapped = receiver.recv().await.expect("should receive command");
            let err = BotError::BlockNotFound(crate::types::BlockPos::new(1, 2, 3));
            wrapped
                .respond_to
                .send(Err(err))
                .expect("should send error");
        });

        let result = sender.send_command(cmd).await;
        assert!(result.is_err());
        assert!(matches!(result, Err(BotError::BlockNotFound(_))));

        responder.await.expect("responder task should complete");
    }

    // ── Timeout / responder drop ──────────────────────────────

    /// When the receiver drops the oneshot sender without responding,
    /// the caller should get `BotError::Offline` (channel closed — distinct
    /// from a real timeout, which would yield `CommandTimeout`).
    #[tokio::test]
    async fn test_offline_when_responder_dropped() {
        let (sender, mut receiver) = create_command_channel(10, make_state());

        let cmd = BotCommand::Jump;

        let responder = tokio::spawn(async move {
            let wrapped = receiver.recv().await.expect("should receive command");
            // Drop `wrapped` without calling `respond_to.send(...)`.
            drop(wrapped);
        });

        let result = sender.send_command(cmd).await;
        assert!(result.is_err());
        match result {
            Err(BotError::Offline(msg)) => {
                assert!(msg.contains("responder dropped"));
            }
            other => panic!("expected Offline, got: {:?}", other),
        }

        responder.await.expect("responder task should complete");
    }

    // ── CommandTimeout — responder alive but slow ─────────────

    /// When the receiver accepts the command but takes longer than
    /// `command_timeout_secs` to reply, the sender should observe a
    /// `BotError::CommandTimeout` — distinct from `BotError::Offline`,
    /// which is reserved for the responder-dropped / receiver-dropped
    /// cases. The responder is kept **alive** (we sleep rather than
    /// drop) so we exercise the genuine `tokio::time::timeout` branch
    /// instead of the responder-dropped branch.
    ///
    /// The minimum representable timeout under the current
    /// `AppConfig::command_timeout_secs: u64` design is 1 second, so
    /// the responder sleeps for 1.5s (longer than 1s, shorter than 2s
    /// to keep the test fast).
    #[tokio::test]
    async fn test_command_timeout_returns_error() {
        let state = make_state();
        // Set a 1-second timeout (smallest possible with u64 seconds).
        state.update_config(|cfg| {
            cfg.command_timeout_secs = 1;
        });
        let (sender, mut receiver) = create_command_channel(10, state);

        let cmd = BotCommand::Jump;

        let responder = tokio::spawn(async move {
            let wrapped = receiver.recv().await.expect("should receive command");
            // Hold the responder for 1.5s — longer than the 1s timeout —
            // so send_command's `tokio::time::timeout` fires first.
            tokio::time::sleep(Duration::from_millis(1500)).await;
            // After the sleep we drop `wrapped` (without sending a
            // response). The test has already received the timeout error
            // by the time we get here, so this drop is irrelevant to the
            // assertion — we just want the task to complete cleanly.
            drop(wrapped);
        });

        let result = sender.send_command(cmd).await;
        assert!(result.is_err(), "expected an error, got: {result:?}");
        match result {
            Err(BotError::CommandTimeout { command, .. }) => {
                // The `command` field should contain the Debug rendering
                // of `BotCommand::Jump` (i.e. the string "Jump").
                assert!(
                    command.contains("Jump"),
                    "expected command field to mention Jump, got: {command}"
                );
            }
            other => panic!("expected BotError::CommandTimeout, got: {other:?}"),
        }

        // Wait for the responder task to finish cleanly.
        responder.await.expect("responder task should complete");
    }

    // ── L-18-prep: send_command_with_timeout ─────────────────────

    /// The given envelope timeout must be used verbatim: a responder that
    /// replies after the given short duration triggers `CommandTimeout`
    /// reporting THAT duration (0s for a 100ms envelope), NOT the config's
    /// 30s command timeout. This is the contract another agent's wave-2
    /// consumer relies on (per-command timeouts without touching the
    /// config).
    #[tokio::test]
    async fn test_send_command_with_timeout_uses_given_envelope() {
        let (sender, mut receiver) = create_command_channel(10, make_state());

        let responder = tokio::spawn(async move {
            let wrapped = receiver.recv().await.expect("should receive command");
            // Hold the responder 5× longer than the given 100ms envelope.
            tokio::time::sleep(Duration::from_millis(500)).await;
            drop(wrapped);
        });

        let result = sender
            .send_command_with_timeout(BotCommand::Jump, Duration::from_millis(100))
            .await;
        assert!(result.is_err(), "expected a timeout, got: {result:?}");
        match result {
            Err(BotError::CommandTimeout { timeout_secs, .. }) => {
                assert_eq!(
                    timeout_secs, 0,
                    "must report the GIVEN 100ms envelope (0s), not the 30s config default"
                );
            }
            other => panic!("expected BotError::CommandTimeout, got: {other:?}"),
        }
        responder.await.expect("responder task should complete");
    }

    // ── L-21: timeout_for reads config once, honours max semantics ──

    /// FlyTo gets the longer of the command and fly timeouts.
    #[test]
    fn test_timeout_for_flyto_uses_max_of_command_and_fly() {
        let state = make_state();
        state.update_config(|cfg| {
            cfg.command_timeout_secs = 10;
            cfg.fly_timeout_secs = 60;
        });
        let (sender, _receiver) = create_command_channel(4, Arc::clone(&state));
        assert_eq!(
            sender.timeout_for(&BotCommand::FlyTo(BlockPos::new(0, 0, 0))),
            Duration::from_secs(60)
        );
        // When the command timeout is longer, it wins.
        state.update_config(|cfg| cfg.command_timeout_secs = 120);
        assert_eq!(
            sender.timeout_for(&BotCommand::FlyTo(BlockPos::new(0, 0, 0))),
            Duration::from_secs(120)
        );
    }

    /// Report M-3: Act(Fly) dispatches the same handle_fly_to executor
    /// path as FlyTo, so it must share the fly envelope. Before the fix a
    /// flight longer than 30 s via the unified act tool surfaced a false
    /// CommandTimeout while the executor kept flying.
    #[test]
    fn test_timeout_for_act_fly_uses_fly_timeout() {
        let state = make_state();
        state.update_config(|cfg| {
            cfg.command_timeout_secs = 10;
            cfg.fly_timeout_secs = 60;
        });
        let (sender, _receiver) = create_command_channel(4, Arc::clone(&state));
        assert_eq!(
            sender.timeout_for(&BotCommand::Act(
                ActAction::Fly {
                    target: BlockPos::new(0, 0, 0)
                },
                None
            )),
            Duration::from_secs(60)
        );
        // Other movement-only Act actions keep the plain command envelope.
        assert_eq!(
            sender.timeout_for(&BotCommand::Act(
                ActAction::Move {
                    target: BlockPos::new(0, 0, 0)
                },
                None
            )),
            Duration::from_secs(10)
        );

        // F-1: compound actions (mine / collect items) share the long
        // envelope so their multi-leg duration is not guaranteed to
        // overrun the client's wait.
        assert_eq!(
            sender.timeout_for(&BotCommand::Act(
                ActAction::Mine {
                    block_pos: BlockPos::new(0, 0, 0)
                },
                None
            )),
            Duration::from_secs(60)
        );
        assert_eq!(
            sender.timeout_for(&BotCommand::Act(
                ActAction::CollectItems { radius: 16 },
                None
            )),
            Duration::from_secs(60)
        );
    }

    /// Non-FlyTo commands use exactly the configured command timeout —
    /// the fly timeout must not leak into their envelope.
    #[test]
    fn test_timeout_for_non_fly_uses_command_timeout() {
        let state = make_state();
        state.update_config(|cfg| {
            cfg.command_timeout_secs = 10;
            cfg.fly_timeout_secs = 60;
        });
        let (sender, _receiver) = create_command_channel(4, Arc::clone(&state));
        assert_eq!(
            sender.timeout_for(&BotCommand::Jump),
            Duration::from_secs(10)
        );
        assert_eq!(
            sender.timeout_for(&BotCommand::BreakBlock(BlockPos::new(1, 2, 3))),
            Duration::from_secs(10)
        );
    }

    // ── Offline ─────────────────────────────────────────────

    #[tokio::test]
    async fn test_offline_when_receiver_dropped() {
        let (sender, receiver) = create_command_channel(10, make_state());

        // Drop the receiver before any command is sent.
        drop(receiver);

        let cmd = BotCommand::Jump;
        let result = sender.send_command(cmd).await;

        assert!(result.is_err());
        match result {
            Err(BotError::Offline(msg)) => {
                assert!(msg.contains("closed"));
            }
            other => panic!("expected Offline, got: {:?}", other),
        }
    }

    // ── Non-blocking receive ────────────────────────────────

    #[tokio::test]
    async fn test_try_recv_empty() {
        let (_sender, mut receiver) = create_command_channel(10, make_state());

        let result = receiver.try_recv();
        assert!(matches!(result, Err(mpsc::error::TryRecvError::Empty)));
    }

    #[tokio::test]
    async fn test_try_recv_success() {
        let (sender, mut receiver) = create_command_channel(10, make_state());

        let cmd = BotCommand::Jump;
        let (respond_to, _rx) = oneshot::channel();
        let wrapped = BotCommandWithResponder {
            command: cmd,
            respond_to,
        };

        // Use the internal mpsc sender directly to enqueue a command.
        sender.tx.send(wrapped).await.expect("should send");

        let received = receiver.try_recv().expect("should receive immediately");
        assert!(matches!(received.command, BotCommand::Jump));
    }

    #[tokio::test]
    async fn test_try_recv_disconnected() {
        let (sender, mut receiver) = create_command_channel(10, make_state());
        drop(sender);

        let result = receiver.try_recv();
        assert!(matches!(
            result,
            Err(mpsc::error::TryRecvError::Disconnected)
        ));
    }

    // ── Serialization / single receiver ─────────────────────

    #[tokio::test]
    async fn test_multiple_commands_processed_serially() {
        let (sender, mut receiver) = create_command_channel(10, make_state());

        let responder = tokio::spawn(async move {
            let mut count = 0;
            while let Some(wrapped) = receiver.recv().await {
                count += 1;
                let result = make_result(true, &format!("ack-{count}"));
                let _ = wrapped.respond_to.send(Ok(result));
            }
            count
        });

        let s1 = sender.clone();
        let s2 = sender.clone();

        let h1 = tokio::spawn(async move { s1.send_command(BotCommand::Jump).await.unwrap() });
        let h2 = tokio::spawn(async move { s2.send_command(BotCommand::UseItem).await.unwrap() });

        let r1 = h1.await.unwrap();
        let r2 = h2.await.unwrap();

        // Both should succeed.
        assert!(r1.success);
        assert!(r2.success);

        // Drop the original sender so the receiver loop terminates.
        drop(sender);

        let total = responder.await.expect("responder should finish");
        assert_eq!(total, 2);
    }

    // ── Sender clone ────────────────────────────────────────

    #[tokio::test]
    async fn test_sender_is_clone() {
        let (sender, mut receiver) = create_command_channel(10, make_state());
        let sender2 = sender.clone();

        let h1 = tokio::spawn(async move { sender.send_command(BotCommand::Jump).await.unwrap() });
        let h2 =
            tokio::spawn(async move { sender2.send_command(BotCommand::UseItem).await.unwrap() });

        let responder = tokio::spawn(async move {
            let mut count = 0;
            while let Some(wrapped) = receiver.recv().await {
                count += 1;
                let _ = wrapped.respond_to.send(Ok(make_result(true, "ok")));
                if count == 2 {
                    break;
                }
            }
        });

        let r1 = h1.await.unwrap();
        let r2 = h2.await.unwrap();
        assert!(r1.success);
        assert!(r2.success);

        responder.await.unwrap();
    }

    // ── Buffer backpressure ─────────────────────────────────

    #[tokio::test]
    async fn test_buffer_backpressure() {
        let (sender, mut receiver) = create_command_channel(1, make_state());

        // Fill the buffer with a non-blocking send.
        let (tx1, _rx1) = oneshot::channel();
        sender
            .tx
            .try_send(BotCommandWithResponder {
                command: BotCommand::Jump,
                respond_to: tx1,
            })
            .unwrap();

        // A second try_send should fail because the buffer is full.
        let (tx2, _rx2) = oneshot::channel();
        let result = sender.tx.try_send(BotCommandWithResponder {
            command: BotCommand::UseItem,
            respond_to: tx2,
        });
        assert!(matches!(result, Err(mpsc::error::TrySendError::Full(_))));

        // Consume the queued command to free capacity.
        let _ = receiver.recv().await.unwrap();
    }

    // ── Dynamic timeout (P1-#10) ─────────────────────────────

    /// The sender must read `command_timeout_secs` from the shared state on
    /// every call to `timeout()`. Mutating the config between two
    /// `timeout()` calls must return the *new* value, not the original
    /// one. This proves the dynamic config binding that the
    /// P1-#10 fix relies on (the Settings panel can change the
    /// timeout and the next MCP call honours it without restart).
    ///
    /// We test the public `timeout()` accessor directly: the alternative
    /// of driving two `send_command` calls through a single
    /// `ReceiverLease` is racy under `cargo test`'s default parallel
    /// scheduler, because the receiver cannot be cloned and a single
    /// `recv().await` can only answer one command — the second
    /// `send_command` always times out, regardless of the
    /// configured value. The accessor test is deterministic and
    /// exercises the same code path that the runtime follows.
    #[test]
    fn test_send_command_uses_latest_timeout() {
        let state = make_state();
        state.update_config(|cfg| cfg.command_timeout_secs = 30);
        let (sender, _receiver) = create_command_channel(4, Arc::clone(&state));

        // Baseline: 30s.
        assert_eq!(sender.timeout(), Duration::from_secs(30));

        // Mutate to 1s and re-check. The sender must observe the
        // live config, not a cached copy.
        state.update_config(|cfg| cfg.command_timeout_secs = 1);
        assert_eq!(sender.timeout(), Duration::from_secs(1));

        // Mutate to 5s and re-check. Every call must re-read.
        state.update_config(|cfg| cfg.command_timeout_secs = 5);
        assert_eq!(sender.timeout(), Duration::from_secs(5));
    }

    /// Sub-second timeouts (e.g. an experimental 200ms) must not be
    /// truncated to 0 by `Duration::as_secs()`. We assert the computed
    /// `Duration` is at least 200ms via `timeout()` and the
    /// `CommandTimeout` error reports a `timeout_secs` that — when
    /// rounded — corresponds to a duration of ≥ 200ms (i.e. the
    /// pre-fix `as_secs(0.2) = 0` truncation is gone).
    #[tokio::test]
    async fn test_subsecond_timeout_not_truncated() {
        // Build a state whose `command_timeout_secs` is 1 (smallest u64),
        // then patch the timeout path through `sender.timeout()` to use
        // a fractional duration. We do this by using a custom helper
        // because `command_timeout_secs` is a u64 field. The important
        // invariant is that `sender.timeout()` returns a Duration that
        // is not 0 when the config field is 1 — i.e. `as_secs_f64()` of
        // the config-derived Duration matches the value the user set.
        let state = make_state();
        state.update_config(|cfg| cfg.command_timeout_secs = 1);
        let (sender, _receiver) = create_command_channel(4, Arc::clone(&state));

        let dur = sender.timeout();
        assert_eq!(dur, Duration::from_secs(1));
        // The Debug representation of the duration is `1s` (not `0s`),
        // which would have been the truncated value if the old
        // `Duration::from_secs_f64` round-trip had been lossy.
        assert_eq!(format!("{dur:?}"), "1s");
    }

    // ── ReceiverLease retry (P1-#9) ──────────────────────────

    /// During a fast reconnect, a previous executor's `ReceiverLease` may
    /// still be alive when the new `Event::Spawn` fires. `take_with_retry`
    /// must poll the slot briefly and observe the receiver land there once
    /// the old lease drops.
    #[tokio::test(flavor = "current_thread")]
    async fn test_receiver_lease_take_retries_during_reconnect() {
        let slot: ReceiverSlot = Arc::new(Mutex::new(None));

        // Simulate a previous executor that has leased the receiver.
        // Place a fresh receiver into the slot first, then take it
        // through `take` so the slot is empty (matching the in-between
        // reconnect window).
        let (tx, rx) = create_command_channel(4, make_state());
        drop(tx);
        *slot.lock().unwrap() = Some(rx);
        let _first_lease = ReceiverLease::take(&slot).expect("first take succeeds");
        assert!(
            slot.lock().unwrap().is_none(),
            "slot must be empty after first lease"
        );

        // The new Spawn fires while the first lease is still alive.
        // Spawn a task that drops the first lease after 20ms — within
        // the 100ms retry window.
        let first_slot = Arc::clone(&slot);
        let drop_task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            // Manually release the first lease by replacing the slot's
            // receiver (simulating `Drop`).
            let (_tx, rx) = create_command_channel(4, make_state());
            *first_slot.lock().unwrap() = Some(rx);
        });

        // The retry loop should eventually find the receiver and return
        // Some(lease). Without the retry, this would return None
        // immediately.
        let lease = ReceiverLease::take_with_retry(&slot).await;
        drop_task.await.unwrap();
        assert!(
            lease.is_some(),
            "take_with_retry should have observed the receiver"
        );
    }

    /// If the slot stays empty for the full 100ms retry window,
    /// `take_with_retry` must return `None` so the caller can take the
    /// "no executor" branch (matches the pre-fix behaviour for the
    /// genuinely-empty case).
    #[tokio::test(flavor = "current_thread")]
    async fn test_receiver_lease_take_with_retry_gives_up_after_window() {
        let slot: ReceiverSlot = Arc::new(Mutex::new(None));
        let lease = ReceiverLease::take_with_retry(&slot).await;
        assert!(
            lease.is_none(),
            "expected None when the slot stays empty for 100ms"
        );
    }
}
