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
use crate::command_validate::validate_act_action;
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
    // Validate the action payload before doing any state checks or sending
    // the command. This catches bad inputs (out-of-range Y, oversized
    // entity_id) at the MCP layer so callers get a fast `InvalidParams`
    // error rather than waiting for the bot executor to fail.
    validate_act_action(&input.action)?;

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
        let (sender, _receiver) = create_command_channel(4, Arc::clone(&state));
        (state, sender)
    }

    fn make_online(state: &SharedState) {
        state.set_online(true);
    }

    /// Create a channel where the receiver echoes back a successful BotResult
    /// carrying the action's debug string.
    fn make_echo_channel() -> (Arc<SharedState>, BotCommandSender) {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, mut receiver) = create_command_channel(4, Arc::clone(&state));

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
        let (sender, mut receiver) = create_command_channel(4, Arc::clone(&state));

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
        let (sender, mut receiver) = create_command_channel(4, Arc::clone(&state));

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

    // ── P0-#2: input validation ────────────────────────────────────────────

    /// `handle_act` must reject invalid `ActAction` payloads with
    /// `InvalidParams` before any state lookups or command dispatch.
    #[tokio::test]
    async fn test_act_input_validation() {
        // Y far above the build height must be rejected.
        let bad_y = ActInput {
            action: ActAction::Move {
                target: BlockPos::new(0, 9999, 0),
            },
        };
        let result = handle_act(
            &{
                let s = SharedState::new(AppConfig::default());
                s.set_online(true);
                Arc::new(s)
            },
            &{
                let s = SharedState::new(AppConfig::default());
                let (tx, _rx) = create_command_channel(4, Arc::new(s));
                tx
            },
            bad_y,
        )
        .await;
        assert!(
            matches!(result, Err(BotError::InvalidParams(_))),
            "y=9999 should be rejected, got {result:?}"
        );

        // entity_id above i32::MAX must be rejected (Attack variant).
        let bad_attack = ActInput {
            action: ActAction::Attack {
                entity_id: u32::MAX,
            },
        };
        let result = handle_act(
            &{
                let s = SharedState::new(AppConfig::default());
                s.set_online(true);
                Arc::new(s)
            },
            &{
                let s = SharedState::new(AppConfig::default());
                let (tx, _rx) = create_command_channel(4, Arc::new(s));
                tx
            },
            bad_attack,
        )
        .await;
        assert!(
            matches!(result, Err(BotError::InvalidParams(_))),
            "Attack entity_id=u32::MAX should be rejected, got {result:?}"
        );
    }

    /// Sanity check that the validation runs before the offline check: a
    /// valid action against an offline bot still produces an `Offline`
    /// error (not a validation error), proving the validation succeeded.
    #[tokio::test]
    async fn test_act_input_validation_passes_before_offline_check() {
        let (state, sender) = setup(); // offline
        let input = ActInput {
            action: ActAction::Move {
                target: BlockPos::new(0, 64, 0), // y = 64 is valid
            },
        };
        let result = handle_act(&state, &sender, input).await;
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    /// Alias of `test_act_input_validation`'s Attack branch: the MCP layer
    /// must reject `Attack { entity_id: u32::MAX }` with `InvalidParams`
    /// **before** checking online status or sending to the bot. A
    /// borderline `i32::MAX` value must be accepted (passes through to the
    /// bot layer), and `i32::MAX + 1` must be rejected.
    #[tokio::test]
    async fn test_handle_act_attack_entity_id_above_i32_max_rejected() {
        // u32::MAX is well above i32::MAX — must be rejected.
        let (state, sender) = setup();
        state.set_online(true);
        let input = ActInput {
            action: ActAction::Attack {
                entity_id: u32::MAX,
            },
        };
        let result = handle_act(&state, &sender, input).await;
        match result {
            Err(BotError::InvalidParams(msg)) => {
                assert!(
                    msg.contains("i32::MAX") || msg.contains("entity_id"),
                    "msg should mention i32::MAX / entity_id, got: {msg}"
                );
            }
            other => panic!("expected InvalidParams for entity_id=u32::MAX, got {other:?}"),
        }

        // i32::MAX is the largest accepted value — must be accepted at the
        // MCP layer (the bot layer decides what to do with it). Use the
        // echo channel so the sender can deliver and reply.
        let (state2, sender2) = make_echo_channel();
        state2.set_online(true);
        let ok_input = ActInput {
            action: ActAction::Attack {
                entity_id: i32::MAX as u32,
            },
        };
        let ok_result = handle_act(&state2, &sender2, ok_input).await;
        assert!(
            ok_result.is_ok(),
            "entity_id=i32::MAX should pass MCP validation, got {:?}",
            ok_result
        );

        // i32::MAX + 1 — must be rejected (one past the bound).
        let (state3, sender3) = setup();
        state3.set_online(true);
        let over_input = ActInput {
            action: ActAction::Attack {
                entity_id: (i32::MAX as u32) + 1,
            },
        };
        let over_result = handle_act(&state3, &sender3, over_input).await;
        assert!(
            matches!(over_result, Err(BotError::InvalidParams(_))),
            "entity_id=i32::MAX+1 should be rejected, got {over_result:?}"
        );
    }
}
