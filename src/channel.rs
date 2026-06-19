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
use crate::types::{BotCommand, BotResult};

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
#[derive(Debug, Clone)]
pub struct BotCommandSender {
    tx: mpsc::Sender<BotCommandWithResponder>,
    /// Per-sender timeout for awaiting a command response. Defaults to 30s;
    /// override with [`with_timeout`](Self::with_timeout).
    timeout: Duration,
}

impl BotCommandSender {
    /// Override the response timeout (e.g. from `AppConfig::command_timeout_secs`).
    ///
    /// Consumes and returns `self` for one-shot configuration at construction.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Send a command to the bot and await the response.
    ///
    /// # Errors
    /// - `BotError::Offline` if the receiver has been dropped.
    /// - `BotError::CommandTimeout` if no response arrives within
    ///   [`BotCommandSender::timeout`] or if the responder side drops the
    ///   oneshot without sending.
    pub async fn send_command(&self, cmd: BotCommand) -> Result<BotResult, BotError> {
        let (respond_to, rx) = oneshot::channel();
        let cmd_str = format!("{:?}", cmd);
        let wrapped = BotCommandWithResponder {
            command: cmd,
            respond_to,
        };

        trace!(command = %cmd_str, "sending bot command");

        if self.tx.send(wrapped).await.is_err() {
            error!("bot command channel closed — receiver dropped");
            return Err(BotError::Offline("bot command channel closed".into()));
        }

        let timeout_secs = self.timeout.as_secs();
        match timeout(self.timeout, rx).await {
            Ok(Ok(result)) => {
                debug!(command = %cmd_str, "bot command completed");
                result
            }
            Ok(Err(_)) => {
                warn!(command = %cmd_str, "bot command responder dropped without reply");
                Err(BotError::CommandTimeout {
                    command: cmd_str,
                    timeout_secs,
                })
            }
            Err(_) => {
                error!(command = %cmd_str, timeout_secs, "bot command timed out");
                Err(BotError::CommandTimeout {
                    command: cmd_str,
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
/// The sender defaults to a 30s response timeout; use
/// [`BotCommandSender::with_timeout`] to override it.
pub fn create_command_channel(buffer: usize) -> (BotCommandSender, BotCommandReceiver) {
    let (tx, rx) = mpsc::channel(buffer);
    (
        BotCommandSender {
            tx,
            timeout: Duration::from_secs(30),
        },
        BotCommandReceiver { rx },
    )
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
        let mut guard = slot.lock().expect("command receiver slot mutex poisoned");
        guard.take().map(|rx| Self {
            slot: Arc::clone(slot),
            receiver: Some(rx),
        })
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
            *self
                .slot
                .lock()
                .expect("command receiver slot mutex poisoned") = Some(rx);
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::BlockPos;

    // ── Helpers ─────────────────────────────────────────────

    fn make_result(success: bool, message: &str) -> BotResult {
        BotResult {
            success,
            message: message.into(),
            data: None,
        }
    }

    // ── Command flow ────────────────────────────────────────

    #[tokio::test]
    async fn test_command_flow_success() {
        let (sender, mut receiver) = create_command_channel(10);

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
        let (sender, mut receiver) = create_command_channel(10);

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
    /// the caller should get `BotError::CommandTimeout`.
    #[tokio::test]
    async fn test_timeout_when_responder_dropped() {
        let (sender, mut receiver) = create_command_channel(10);

        let cmd = BotCommand::Jump;

        let responder = tokio::spawn(async move {
            let wrapped = receiver.recv().await.expect("should receive command");
            // Drop `wrapped` without calling `respond_to.send(...)`.
            drop(wrapped);
        });

        let result = sender.send_command(cmd).await;
        assert!(result.is_err());
        match result {
            Err(BotError::CommandTimeout {
                command,
                timeout_secs,
            }) => {
                assert!(command.contains("Jump"));
                assert_eq!(timeout_secs, 30);
            }
            other => panic!("expected CommandTimeout, got: {:?}", other),
        }

        responder.await.expect("responder task should complete");
    }

    // ── Offline ─────────────────────────────────────────────

    #[tokio::test]
    async fn test_offline_when_receiver_dropped() {
        let (sender, receiver) = create_command_channel(10);

        // Drop the receiver before any command is sent.
        drop(receiver);

        let cmd = BotCommand::QuerySelfInfo;
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
        let (_sender, mut receiver) = create_command_channel(10);

        let result = receiver.try_recv();
        assert!(matches!(result, Err(mpsc::error::TryRecvError::Empty)));
    }

    #[tokio::test]
    async fn test_try_recv_success() {
        let (sender, mut receiver) = create_command_channel(10);

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
        let (sender, mut receiver) = create_command_channel(10);
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
        let (sender, mut receiver) = create_command_channel(10);

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
        let (sender, mut receiver) = create_command_channel(10);
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
        let (sender, mut receiver) = create_command_channel(1);

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
}
