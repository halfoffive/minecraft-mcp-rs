//! Shared MCP-tool plumbing: the single online gate and the single
//! send-and-serialize tail.
//!
//! Every action tool used to hand-roll two identical snippets — the offline
//! rejection (`if !state.is_online() { return Err(BotError::Offline(...)) }`)
//! and the command dispatch tail (`match sender.send_command(cmd).await {
//! Ok(result) => serde_json::to_string(&result)..., Err(e) => Err(e) }`). The
//! copies had already drifted three different offline messages across one
//! tool surface ("Bot is currently offline" / "Bot is offline" / "Bot is not
//! connected to a server"); both helpers here are now the ONLY sanctioned
//! form, so the wording and the error mapping can never diverge again.

use std::sync::Arc;

use crate::channel::BotCommandSender;
use crate::error::BotError;
use crate::state::SharedState;
use crate::types::{BotCommand, BotResult};

/// The one offline message every tool surfaces.
///
/// Query tools historically said "Bot is currently offline", settings/chat
/// tools "Bot is offline", and the remaining action tools "Bot is not
/// connected to a server" — same condition, three strings. This constant is
/// the single source of truth; do not inline the text at call sites.
pub const OFFLINE_MESSAGE: &str = "Bot is currently offline";

/// Reject the call when the bot is not connected to a Minecraft server.
///
/// Maps to [`BotError::Offline`] (JSON-RPC -32000, `reason:
/// bot_disconnected`) so clients can branch on `error.code` alone.
#[inline]
pub fn require_online(state: &Arc<SharedState>) -> Result<(), BotError> {
    if state.is_online() {
        Ok(())
    } else {
        Err(BotError::Offline(OFFLINE_MESSAGE.to_string()))
    }
}

/// Send a command through the channel and serialize the executor's
/// [`BotResult`] as the tool's JSON response string.
///
/// The serialization failure (which should be unreachable for
/// `BotResult`-shaped data) surfaces as [`BotError::Internal`]; a transport
/// or executor error passes through unchanged so its structured
/// `reason`/`retryable` payload reaches the client intact.
pub async fn send_and_serialize(
    sender: &BotCommandSender,
    cmd: BotCommand,
) -> Result<String, BotError> {
    let result: BotResult = sender.send_command(cmd).await?;
    serde_json::to_string(&result)
        .map_err(|e| BotError::Internal(format!("Serialization error: {e}")))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::create_command_channel;
    use crate::config::AppConfig;

    #[test]
    fn test_require_online_rejects_offline_with_canonical_message() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        match require_online(&state) {
            Err(BotError::Offline(msg)) => assert_eq!(msg, OFFLINE_MESSAGE),
            other => panic!("expected Offline, got {other:?}"),
        }
    }

    #[test]
    fn test_require_online_accepts_online() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        state.set_online(true);
        assert!(require_online(&state).is_ok());
    }

    /// End-to-end through a real channel: the serialized response parses back
    /// into the executor's `BotResult`.
    #[tokio::test]
    async fn test_send_and_serialize_round_trips_bot_result() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, mut receiver) = create_command_channel(4, Arc::clone(&state));
        tokio::spawn(async move {
            while let Some(wrapped) = receiver.recv().await {
                let _ = wrapped.respond_to.send(Ok(BotResult {
                    success: true,
                    message: "jumped".into(),
                    data: None,
                }));
            }
        });

        let raw = send_and_serialize(&sender, BotCommand::Jump)
            .await
            .expect("send should succeed");
        let v: serde_json::Value = serde_json::from_str(&raw).expect("response must be valid JSON");
        assert_eq!(v["success"], true);
        assert_eq!(v["message"], "jumped");
    }

    /// Executor errors pass through untouched (no wrapping in Internal).
    #[tokio::test]
    async fn test_send_and_serialize_passes_executor_error_through() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, mut receiver) = create_command_channel(4, Arc::clone(&state));
        tokio::spawn(async move {
            if let Some(wrapped) = receiver.recv().await {
                let _ = wrapped.respond_to.send(Err(BotError::BlockNotFound(
                    crate::types::BlockPos::new(1, 2, 3),
                )));
            }
        });

        let err = send_and_serialize(&sender, BotCommand::Jump)
            .await
            .expect_err("executor error must propagate");
        assert!(
            matches!(err, BotError::BlockNotFound(_)),
            "expected BlockNotFound passthrough, got {err:?}"
        );
    }
}
