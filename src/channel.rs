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

    /// Drain every command currently buffered in the channel, replying to
    /// each sender with an honest [`BotError::Offline`] instead of executing
    /// it (2026-08-29 review).
    ///
    /// Commands that were sent while no executor held the receiver (the bot
    /// was offline, or the previous session's executor was aborted
    /// mid-flight) sit in the mpsc buffer with their responders already
    /// gone-or-timing-out. When a NEW session's executor leases the
    /// receiver, executing those stale commands would leak session A's
    /// effects into session B (responses lost either way). Draining at
    /// lease start discards them deterministically and answers every
    /// sender immediately.
    ///
    /// Returns the number of drained commands. Note the millisecond window
    /// between `set_online(true)` (in the spawn handler) and this drain: a
    /// command sent there is drained with the same honest Offline error,
    /// which the client can retry.
    pub(crate) fn drain_stale(&mut self, reason: &str) -> usize {
        let mut drained = 0;
        while let Ok(wrapped) = self.receiver_mut().try_recv() {
            let BotCommandWithResponder { command, respond_to } = wrapped;
            tracing::debug!(command = ?command, "discarding stale cross-session command");
            let _ = respond_to.send(Err(BotError::Offline(format!(
                "{reason}; reconnect and retry"
            ))));
            drained += 1;
        }
        drained
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
    use crate::types::{BlockPos, Direction, GameMode, MaterialTier, ToolType};

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
    /// RED (2026-08-29 review): commands buffered while no executor held
    /// the receiver belong to a previous session. `ReceiverLease::
    /// drain_stale` must discard them and answer every sender with an
    /// immediate honest Offline error instead of letting the new session's
    /// executor run them with responses lost.
    #[tokio::test]
    async fn test_drain_stale_replies_offline_and_empties_queue() {
        let slot: ReceiverSlot = Arc::new(Mutex::new(None));
        let state = make_state();
        let (sender, rx) = create_command_channel(4, Arc::clone(&state));
        *slot.lock().unwrap() = Some(rx);

        let mut lease = ReceiverLease::take(&slot).expect("lease taken");
        assert!(slot.lock().unwrap().is_none());

        // Buffer two stale commands: the senders block on the oneshot
        // reply (no executor is running yet).
        let mut r1 = tokio::spawn({
            let sender = sender.clone();
            async move { sender.send_command(BotCommand::Jump).await }
        });
        let mut r2 = tokio::spawn(async move { sender.send_command(BotCommand::Jump).await });
        // Let both sends land in the buffer.
        tokio::time::sleep(Duration::from_millis(50)).await;

        let drained = lease.drain_stale("stale command from a previous session");
        assert_eq!(drained, 2, "both buffered commands must be drained");

        // Every drained sender gets an immediate honest Offline error.
        for r in [&mut r1, &mut r2] {
            let result = r.await.expect("sender task finished");
            assert!(
                matches!(&result, Err(BotError::Offline(msg)) if msg.contains("stale command")),
                "expected honest Offline error, got: {result:?}"
            );
        }

        // The queue is empty — the executor loop would start clean.
        assert!(
            lease.receiver_mut().try_recv().is_err(),
            "no command may survive the drain"
        );
    }

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

    /// Exhaustive-envelope guard (2026-08-26 review): `timeout_for` classifies
    /// with `matches!` rather than a `match`, so a newly added `BotCommand`
    /// (or `ActAction`) variant would silently take the plain command
    /// envelope. This test-side exhaustive match forces a compile error when
    /// a variant is added without an explicit classification here, and then
    /// asserts the actual `timeout_for` output against it for every variant.
    ///
    /// Long envelope: FlyTo, Act(Fly), plus the compound ops Mine and
    /// CollectItems (F-1). Everything else — including the single-leg
    /// Act(Move/SmartMove/Attack) variants — keeps the base envelope.
    #[test]
    fn test_timeout_envelope_class_is_exhaustive_per_variant() {
        fn expected_long_envelope(cmd: &BotCommand) -> bool {
            match cmd {
                BotCommand::FlyTo(_) => true,
                BotCommand::Act(action, _) => match action {
                    ActAction::Move { .. }
                    | ActAction::SmartMove { .. }
                    | ActAction::Attack { .. } => false,
                    ActAction::Fly { .. }
                    | ActAction::Mine { .. }
                    | ActAction::CollectItems { .. } => true,
                },
                BotCommand::MoveTo(_)
                | BotCommand::WalkDirection(_, _)
                | BotCommand::Jump
                | BotCommand::Teleport(_)
                | BotCommand::BreakBlock(_)
                | BotCommand::PlaceBlock(_, _)
                | BotCommand::UseItemOnBlock(_, _, _)
                | BotCommand::SwitchHotbarSlot(_)
                | BotCommand::DropItem(_, _)
                | BotCommand::MoveItemToHotbar(_, _, _)
                | BotCommand::UseItem
                | BotCommand::UseItemWithSlot(_)
                | BotCommand::EquipTool(_)
                | BotCommand::EquipToolWithMaterial(_, _)
                | BotCommand::OpenContainer(_)
                | BotCommand::TakeFromContainer(_, _)
                | BotCommand::PutIntoContainer(_, _)
                | BotCommand::CloseContainer
                | BotCommand::AttackEntity(_)
                | BotCommand::ShieldBlock(_)
                | BotCommand::SendChat(_)
                | BotCommand::ExecuteCommand(_)
                | BotCommand::SetGameMode(_)
                | BotCommand::QueryInventory
                | BotCommand::SmartMove(_)
                | BotCommand::CollectItems(_) => false,
            }
        }

        let commands: Vec<BotCommand> = vec![
            BotCommand::MoveTo(BlockPos::new(0, 0, 0)),
            BotCommand::WalkDirection(Direction::North, 1),
            BotCommand::Jump,
            BotCommand::Teleport(BlockPos::new(0, 0, 0)),
            BotCommand::BreakBlock(BlockPos::new(0, 0, 0)),
            BotCommand::PlaceBlock(BlockPos::new(0, 0, 0), "stone".into()),
            BotCommand::UseItemOnBlock(BlockPos::new(0, 0, 0), None, None),
            BotCommand::SwitchHotbarSlot(0),
            BotCommand::DropItem(0, 1),
            BotCommand::MoveItemToHotbar(0, "dirt".into(), 1),
            BotCommand::UseItem,
            BotCommand::UseItemWithSlot(0),
            BotCommand::EquipTool(ToolType::Pickaxe),
            BotCommand::EquipToolWithMaterial(ToolType::Pickaxe, MaterialTier::Diamond),
            BotCommand::OpenContainer(BlockPos::new(0, 0, 0)),
            BotCommand::TakeFromContainer(0, 1),
            BotCommand::PutIntoContainer(0, 1),
            BotCommand::CloseContainer,
            BotCommand::AttackEntity(0),
            BotCommand::ShieldBlock(true),
            BotCommand::SendChat(String::new()),
            BotCommand::ExecuteCommand(String::new()),
            BotCommand::SetGameMode(GameMode::Survival),
            BotCommand::QueryInventory,
            BotCommand::SmartMove(BlockPos::new(0, 0, 0)),
            BotCommand::FlyTo(BlockPos::new(0, 0, 0)),
            BotCommand::CollectItems(8),
            // One construction per ActAction sub-variant so every nested arm
            // of `expected_long_envelope` is exercised at runtime too.
            BotCommand::Act(
                ActAction::Move {
                    target: BlockPos::new(0, 0, 0),
                },
                None,
            ),
            BotCommand::Act(
                ActAction::SmartMove {
                    target: BlockPos::new(0, 0, 0),
                },
                None,
            ),
            BotCommand::Act(
                ActAction::Fly {
                    target: BlockPos::new(0, 0, 0),
                },
                None,
            ),
            BotCommand::Act(
                ActAction::Mine {
                    block_pos: BlockPos::new(0, 0, 0),
                },
                None,
            ),
            BotCommand::Act(ActAction::Attack { entity_id: 0 }, None),
            BotCommand::Act(ActAction::CollectItems { radius: 8 }, None),
        ];

        let state = make_state();
        state.update_config(|cfg| {
            // Distinct non-default values so neither timeout can accidentally
            // satisfy the other class.
            cfg.command_timeout_secs = 7;
            cfg.fly_timeout_secs = 99;
        });
        let (sender, _receiver) = create_command_channel(4, Arc::clone(&state));

        assert_eq!(
            commands.len(),
            33,
            "28 BotCommand variants + 5 extra per-ActAction constructions; keep \
             this in lock-step with types.rs's 28-variant count"
        );
        for cmd in &commands {
            let want = if expected_long_envelope(cmd) {
                Duration::from_secs(99)
            } else {
                Duration::from_secs(7)
            };
            assert_eq!(
                sender.timeout_for(cmd),
                want,
                "envelope mismatch for {cmd:?}"
            );
        }
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
        // 2026-08-29 review rewrite: the old body only set
        // command_timeout_secs=1 and asserted `sender.timeout() == 1s` —
        // it never exercised a sub-second envelope at all (the config
        // field is u64 seconds, so send_command cannot produce one; only
        // `send_command_with_timeout` can). Send through a 200 ms envelope
        // with a receiver that never replies: the send must fail with
        // CommandTimeout after roughly 200 ms, not instantly (a 0-second
        // truncation) and not after the full 1 s config envelope.
        let state = make_state();
        state.update_config(|cfg| cfg.command_timeout_secs = 1);
        let (sender, mut receiver) = create_command_channel(4, Arc::clone(&state));

        // Hold the receiver alive but never answer.
        let holder = tokio::spawn(async move {
            // Keep the command buffered until the envelope fires.
            let _command = receiver.recv().await;
            // Hold the wrapped command (and its responder) for 1 s so the
            // sender's envelope — not a responder drop — decides.
            tokio::time::sleep(Duration::from_secs(1)).await;
        });

        let started = std::time::Instant::now();
        let result = sender
            .send_command_with_timeout(BotCommand::Jump, Duration::from_millis(200))
            .await;
        let elapsed = started.elapsed();

        match result {
            Err(BotError::CommandTimeout { timeout_secs, .. }) => {
                assert_eq!(timeout_secs, 0, "200 ms envelope rounds to 0 secs");
            }
            other => panic!("expected CommandTimeout, got: {other:?}"),
        }
        assert!(
            elapsed >= Duration::from_millis(180),
            "envelope must not be truncated to 0 (elapsed {elapsed:?})"
        );
        assert!(
            elapsed < Duration::from_millis(900),
            "explicit envelope must win over the 1 s config (elapsed {elapsed:?})"
        );
        holder.abort();
    }

    // ── ReceiverLease retry (P1-#9) ──────────────────────────

    /// During a fast reconnect, a previous executor's `ReceiverLease` may
    /// still be alive when the new `Event::Spawn` fires. `take_with_retry`
    /// must poll the slot briefly and observe the receiver land there once
    /// the old lease drops.
    ///
    /// 2026-08-29 review rewrite: the old body simulated the drop by
    /// inserting a brand-new receiver from a DIFFERENT channel, which
    /// proves only "poll until the slot is non-empty" — not the actual
    /// hand-off invariant (the SAME receiver returns via lease Drop and
    /// the new executor acquires it). The task now drops the real lease.
    #[tokio::test(flavor = "current_thread")]
    async fn test_receiver_lease_take_retries_during_reconnect() {
        let slot: ReceiverSlot = Arc::new(Mutex::new(None));

        // A previous executor has leased the receiver: place it in the
        // slot, take it out through `take`, and remember WHICH receiver
        // the lease holds.
        let (tx, rx) = create_command_channel(4, make_state());
        let tx = Arc::new(tx);
        *slot.lock().unwrap() = Some(rx);
        let first_lease = ReceiverLease::take(&slot).expect("first take succeeds");
        assert!(
            slot.lock().unwrap().is_none(),
            "slot must be empty after first lease"
        );

        // The new Spawn fires while the first lease is still alive. The
        // background task drops the REAL lease after 20 ms — within the
        // 100 ms retry window — which returns the same receiver to the
        // slot via `ReceiverLease::drop`.
        let drop_task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            drop(first_lease);
        });

        // The retry loop should eventually find the receiver and return
        // Some(lease). Without the retry, this would return None
        // immediately.
        let lease = ReceiverLease::take_with_retry(&slot).await;
        drop_task.await.unwrap();
        let lease = lease.expect("take_with_retry should have observed the receiver");

        // The re-acquired receiver is the SAME channel: a command sent by
        // the original sender reaches the new executor (drop(tx) below
        // would close a foreign channel instead).
        let mut lease = lease;
        let probe = tokio::spawn(async move {
            tx.send_command(BotCommand::Jump).await
        });
        let wrapped = lease
            .receiver_mut()
            .recv()
            .await
            .expect("original channel still flows through the re-acquired lease");
        assert!(matches!(wrapped.command, BotCommand::Jump));
        // Answer so the probe sender finishes cleanly.
        let _ = wrapped.respond_to.send(Ok(make_result(true, "ok")));
        probe.await.expect("probe task finished").expect("send ok");
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
