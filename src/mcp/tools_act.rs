//! MCP tool for the unified `act` action.
//!
//! The `act` tool accepts an [`ActAction`] enum and dispatches it through the
//! bot command channel as [`BotCommand::Act`]. The bot layer executes the
//! action and returns a serialised [`crate::types::ActResult`] (carrying nearby blocks,
//! entities, and self info) so an LLM can iterate: act → observe → decide.
//!
//! # Parameter structs
//!
//! [`ActAction`] already derives `schemars::JsonSchema` in `types.rs`, and the
//! project shares a single schemars crate instance with rmcp, so we can derive
//! the schema on [`ActInput`] directly.

use std::sync::Arc;

use serde::Deserialize;

use crate::channel::BotCommandSender;
use crate::error::BotError;
use crate::state::SharedState;
use crate::types::{ActAction, BotCommand};

// ── act ─────────────────────────────────────────────────────────────────────

/// Input for the unified `act` MCP tool.
///
/// Wraps a single [`ActAction`] describing the action to execute.
#[derive(Deserialize, rmcp::schemars::JsonSchema)]
pub struct ActInput {
    /// The action to execute. One of: `move`, `smart_move`, `fly`, `mine`,
    /// `attack`, `collect_items`.
    pub action: ActAction,
}

/// Handle the unified `act` MCP tool.
///
/// Checks online status, then sends [`BotCommand::Act`] with the supplied
/// [`ActAction`]. The bot layer returns a [`crate::types::BotResult`] whose
/// `data` field (when present) carries a serialised [`crate::types::ActResult`].
/// The handler forwards the bot's JSON response as a string so the caller can
/// parse the structured payload.
pub async fn handle_act(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    input: ActInput,
) -> Result<String, BotError> {
    if !state.is_online() {
        return Err(BotError::Offline(
            "Bot is not connected to a server".to_string(),
        ));
    }

    let cmd = BotCommand::Act(input.action);
    match sender.send_command(cmd).await {
        Ok(result) => serde_json::to_string(&result)
            .map_err(|e| BotError::Internal(format!("Serialization error: {e}"))),
        Err(e) => Err(BotError::Internal(format!("Command failed: {e}"))),
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use rmcp::schemars::JsonSchema;
    use serde_json::Value;

    use super::*;
    use crate::channel::create_command_channel;
    use crate::config::AppConfig;
    use crate::types::{BlockPos, BotResult};

    fn setup() -> (Arc<SharedState>, BotCommandSender) {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, _receiver) = create_command_channel(4);
        (state, sender)
    }

    fn make_online(state: &SharedState) {
        state.set_online(true);
    }

    /// Create a channel where the receiver echoes back a successful BotResult
    /// carrying the action's debug string.
    fn make_echo_channel() -> (Arc<SharedState>, BotCommandSender) {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, mut receiver) = create_command_channel(4);

        tokio::spawn(async move {
            while let Some(wrapped) = receiver.recv().await {
                let msg = format!("executed: {:?}", wrapped.command);
                let _ = wrapped.respond_to.send(Ok(BotResult {
                    success: true,
                    message: msg,
                    data: None,
                }));
            }
        });

        (state, sender)
    }

    // ── act: Move ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_act_move() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        make_online(&state);
        let (sender, mut receiver) = create_command_channel(4);

        let responder = tokio::spawn(async move {
            let wrapped = receiver.recv().await.expect("should receive command");
            match wrapped.command {
                BotCommand::Act(ActAction::Move { target }) => {
                    assert_eq!(target, BlockPos::new(1, 64, 2));
                }
                other => panic!("expected Act(Move), got: {other:?}"),
            }
            wrapped
                .respond_to
                .send(Ok(BotResult {
                    success: true,
                    message: "moved".into(),
                    data: None,
                }))
                .expect("should respond");
        });

        let input = ActInput {
            action: ActAction::Move {
                target: BlockPos::new(1, 64, 2),
            },
        };
        let result = handle_act(&state, &sender, input).await.unwrap();
        let json: Value = serde_json::from_str(&result).expect("valid JSON");
        assert!(
            json.get("success")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        );
        responder.await.expect("responder should finish");
    }

    // ── act: Mine ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_act_mine() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        make_online(&state);
        let (sender, mut receiver) = create_command_channel(4);

        let responder = tokio::spawn(async move {
            let wrapped = receiver.recv().await.expect("should receive command");
            match wrapped.command {
                BotCommand::Act(ActAction::Mine { block_pos }) => {
                    assert_eq!(block_pos, BlockPos::new(5, 60, -7));
                }
                other => panic!("expected Act(Mine), got: {other:?}"),
            }
            wrapped
                .respond_to
                .send(Ok(BotResult {
                    success: true,
                    message: "mined".into(),
                    data: None,
                }))
                .expect("should respond");
        });

        let input = ActInput {
            action: ActAction::Mine {
                block_pos: BlockPos::new(5, 60, -7),
            },
        };
        let result = handle_act(&state, &sender, input).await.unwrap();
        let json: Value = serde_json::from_str(&result).expect("valid JSON");
        assert!(
            json.get("success")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        );
        responder.await.expect("responder should finish");
    }

    // ── act: offline ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_act_offline() {
        let (state, sender) = setup();
        let input = ActInput {
            action: ActAction::Move {
                target: BlockPos::new(0, 64, 0),
            },
        };
        let result = handle_act(&state, &sender, input).await;
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    // ── act: echo channel round-trip ────────────────────────────────────────

    #[tokio::test]
    async fn test_act_echo_roundtrip() {
        let (state, sender) = make_echo_channel();
        make_online(&state);
        let input = ActInput {
            action: ActAction::CollectItems { radius: 8 },
        };
        let result = handle_act(&state, &sender, input).await.unwrap();
        let json: Value = serde_json::from_str(&result).expect("valid JSON");
        assert!(
            json.get("success")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        );
        assert!(json["message"].as_str().unwrap().contains("CollectItems"));
    }

    // ── ActInput schema ────────────────────────────────────────────────────

    #[test]
    fn test_act_input_schema_name() {
        let name = <ActInput as JsonSchema>::schema_name();
        assert_eq!(name, Cow::Borrowed("ActInput"));
    }
}
