//! MCP tools for combat (attack entity, shield block).
//!
//! Each tool validates parameters, checks online status (and entity existence
//! for attacks), then dispatches a [`BotCommand`] through the bot command
//! channel.

use std::sync::Arc;

use serde::Deserialize;
use serde_json::Value;

use crate::channel::BotCommandSender;
use crate::error::BotError;
use crate::state::SharedState;
use crate::types::BotCommand;

// ── attack_entity ───────────────────────────────────────────────────────────

/// Input for the `attack_entity` MCP tool.
#[derive(Deserialize, Default, rmcp::schemars::JsonSchema)]
pub struct AttackEntityInput {
    /// The Minecraft entity ID to attack.
    pub entity_id: u32,
}

/// Handle `attack_entity` MCP tool.
///
/// Verifies the entity exists in the current world snapshot, checks online
/// status, then sends [`BotCommand::AttackEntity`].
pub async fn handle_attack_entity(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    input: AttackEntityInput,
) -> Result<String, BotError> {
    // Verify the entity exists in the current snapshot
    {
        let snap = state.read_snapshot();
        let found = snap.entities.iter().any(|e| e.id == input.entity_id);
        if !found {
            return Err(BotError::InvalidParams(format!(
                "Entity with ID {} not found in current world snapshot",
                input.entity_id
            )));
        }
    }

    if !state.is_online() {
        return Err(BotError::Offline(
            "Bot is not connected to a server".to_string(),
        ));
    }

    let cmd = BotCommand::AttackEntity(input.entity_id);
    match sender.send_command(cmd).await {
        Ok(result) => serde_json::to_string(&result)
            .map_err(|e| BotError::Internal(format!("Serialization error: {e}"))),
        Err(e) => Err(e),
    }
}

// ── shield_block ────────────────────────────────────────────────────────────

/// Input for the `shield_block` MCP tool.
#[derive(Deserialize, Default, rmcp::schemars::JsonSchema)]
pub struct ShieldBlockInput {
    /// True to raise shield (start blocking), false to lower shield (stop blocking).
    pub blocking: bool,
}

/// Handle `shield_block` MCP tool.
///
/// Checks online status, then sends [`BotCommand::ShieldBlock`] with the
/// requested state. `blocking = true` raises the shield (crouch);
/// `blocking = false` lowers the shield (stop crouching). The `blocking`
/// parameter is also included in the response metadata.
pub async fn handle_shield_block(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    input: ShieldBlockInput,
) -> Result<String, BotError> {
    if !state.is_online() {
        return Err(BotError::Offline(
            "Bot is not connected to a server".to_string(),
        ));
    }

    let cmd = BotCommand::ShieldBlock(input.blocking);
    match sender.send_command(cmd).await {
        Ok(result) => {
            let mut json = serde_json::to_value(&result)
                .map_err(|e| BotError::Internal(format!("Serialization error: {e}")))?;
            if let Some(obj) = json.as_object_mut() {
                obj.insert("blocking".to_string(), Value::Bool(input.blocking));
            }
            serde_json::to_string(&json)
                .map_err(|e| BotError::Internal(format!("Serialization error: {e}")))
        }
        Err(e) => Err(e),
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::create_command_channel;
    use crate::config::AppConfig;
    use crate::types::{BlockPos, EntityEntry, WorldSnapshot};

    fn setup() -> (Arc<SharedState>, BotCommandSender) {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, _receiver) = create_command_channel(4, Arc::clone(&state));
        (state, sender)
    }

    fn make_online(state: &SharedState) {
        state.set_online(true);
    }

    /// Create a channel where the receiver echoes back a successful BotResult.
    fn make_echo_channel() -> (Arc<SharedState>, BotCommandSender) {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, mut receiver) = create_command_channel(4, Arc::clone(&state));

        tokio::spawn(async move {
            while let Some(wrapped) = receiver.recv().await {
                let msg = format!("executed: {:?}", wrapped.command);
                let _ = wrapped.respond_to.send(Ok(crate::types::BotResult {
                    success: true,
                    message: msg,
                    data: None,
                }));
            }
        });

        (state, sender)
    }

    /// Populate the world snapshot with a test entity.
    fn add_test_entity(state: &SharedState, id: u32) {
        let snap = WorldSnapshot {
            blocks: vec![],
            entities: vec![EntityEntry {
                id,
                uuid: format!("entity-uuid-{id}"),
                entity_type: "zombie".into(),
                position: BlockPos::new(10, 64, 10),
                display_name: Some("Zombie".into()),
                health: Some(20.0),
            }],
            self_player: crate::types::SelfPlayer {
                uuid: "player-uuid".into(),
                username: "TestBot".into(),
                position: BlockPos::new(0, 64, 0),
                health: 20.0,
                hunger: 20,
                gamemode: crate::types::GameMode::Survival,
                held_item_slot: 0,
                inventory: Vec::new(),
            },
            timestamp: 1,
            chunk_summary: vec![],
            commands_enabled: None,
            ..Default::default()
        };
        state.update_snapshot(snap);
    }

    // ── attack_entity ───────────────────────────────────────────

    #[tokio::test]
    async fn test_attack_entity_offline() {
        let (state, sender) = setup();
        add_test_entity(&state, 42);
        let input = AttackEntityInput { entity_id: 42 };
        let result = handle_attack_entity(&state, &sender, input).await;
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    #[tokio::test]
    async fn test_attack_entity_not_found() {
        let (state, sender) = setup();
        make_online(&state);
        // No entities in default snapshot
        let input = AttackEntityInput { entity_id: 99 };
        let result = handle_attack_entity(&state, &sender, input).await;
        assert!(
            matches!(result, Err(BotError::InvalidParams(ref msg)) if msg.contains("not found"))
        );
    }

    #[tokio::test]
    async fn test_attack_entity_valid() {
        let (state, sender) = make_echo_channel();
        make_online(&state);
        add_test_entity(&state, 42);
        let input = AttackEntityInput { entity_id: 42 };
        let result = handle_attack_entity(&state, &sender, input).await;
        let _: Value = serde_json::from_str(&result.unwrap()).expect("valid JSON");
    }

    // ── shield_block ────────────────────────────────────────────

    #[tokio::test]
    async fn test_shield_block_offline() {
        let (state, sender) = setup();
        let input = ShieldBlockInput { blocking: true };
        let result = handle_shield_block(&state, &sender, input).await;
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    #[tokio::test]
    async fn test_shield_block_start() {
        let (state, sender) = make_echo_channel();
        make_online(&state);
        let input = ShieldBlockInput { blocking: true };
        let result = handle_shield_block(&state, &sender, input).await.unwrap();
        let json: Value = serde_json::from_str(&result).expect("valid JSON");
        assert_eq!(json.get("blocking"), Some(&Value::Bool(true)));
    }

    #[tokio::test]
    async fn test_shield_block_stop() {
        // Verify blocking=false is propagated as BotCommand::ShieldBlock(false).
        let state = Arc::new(SharedState::new(AppConfig::default()));
        make_online(&state);
        let (sender, mut receiver) = create_command_channel(4, Arc::clone(&state));

        let responder = tokio::spawn(async move {
            let wrapped = receiver.recv().await.expect("should receive command");
            assert!(
                matches!(wrapped.command, BotCommand::ShieldBlock(false)),
                "expected ShieldBlock(false), got: {:?}",
                wrapped.command
            );
            wrapped
                .respond_to
                .send(Ok(crate::types::BotResult {
                    success: true,
                    message: "ok".into(),
                    data: None,
                }))
                .expect("should respond");
        });

        let input = ShieldBlockInput { blocking: false };
        let result = handle_shield_block(&state, &sender, input).await.unwrap();
        let json: Value = serde_json::from_str(&result).expect("valid JSON");
        assert!(
            json.get("success")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            "expected success, got: {result}"
        );
        assert_eq!(json.get("blocking"), Some(&Value::Bool(false)));

        responder.await.expect("responder should finish");
    }
}
