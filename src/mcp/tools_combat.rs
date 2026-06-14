//! MCP tools for combat (attack entity, shield block).
//!
//! Each tool validates parameters, checks online status (and entity existence
//! for attacks), then dispatches a [`BotCommand`](crate::types::BotCommand)
//! through the bot command channel.
//!
//! # Parameter structs
//!
//! We implement [`rmcp::schemars::JsonSchema`] manually using schemars v1.2.1
//! API (bundled by rmcp 1.7.0) to avoid version conflicts with the project's
//! schemars v0.8 dependency.

use std::borrow::Cow;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::channel::BotCommandSender;
use crate::state::SharedState;
use crate::types::BotCommand;

// ── Helper ──────────────────────────────────────────────────────────────────

fn schema_from_json(v: Value) -> rmcp::schemars::Schema {
    let map: Map<String, Value> = v.as_object().cloned().unwrap_or_default();
    rmcp::schemars::Schema::from(map)
}

// ── attack_entity ───────────────────────────────────────────────────────────

/// Input for the `attack_entity` MCP tool.
#[derive(Deserialize, Default)]
pub struct AttackEntityInput {
    pub entity_id: u32,
}

impl rmcp::schemars::JsonSchema for AttackEntityInput {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("AttackEntityInput")
    }

    fn json_schema(_gen: &mut rmcp::schemars::SchemaGenerator) -> rmcp::schemars::Schema {
        schema_from_json(json!({
            "type": "object",
            "properties": {
                "entity_id": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "The Minecraft entity ID to attack"
                }
            },
            "required": ["entity_id"],
            "additionalProperties": false
        }))
    }
}

/// Handle `attack_entity` MCP tool.
///
/// Verifies the entity exists in the current world snapshot, checks online
/// status, then sends [`BotCommand::AttackEntity`].
pub async fn handle_attack_entity(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    input: AttackEntityInput,
) -> String {
    // Verify the entity exists in the current snapshot
    {
        let snap = state.read_snapshot();
        let found = snap.entities.iter().any(|e| e.id == input.entity_id);
        if !found {
            return format!(
                r#"{{"success":false,"error":"Entity with ID {} not found in current world snapshot"}}"#,
                input.entity_id
            );
        }
    }

    if !state.is_online() {
        return r#"{"success":false,"error":"Bot is not connected to a server"}"#.to_string();
    }

    let cmd = BotCommand::AttackEntity(input.entity_id);
    match sender.send_command(cmd).await {
        Ok(result) => serde_json::to_string(&result).unwrap_or_else(|e| {
            format!(r#"{{"success":false,"error":"Serialization error: {e}"}}"#)
        }),
        Err(e) => format!(r#"{{"success":false,"error":"{e}"}}"#),
    }
}

// ── shield_block ────────────────────────────────────────────────────────────

/// Input for the `shield_block` MCP tool.
#[derive(Deserialize, Default)]
pub struct ShieldBlockInput {
    pub blocking: bool,
}

impl rmcp::schemars::JsonSchema for ShieldBlockInput {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("ShieldBlockInput")
    }

    fn json_schema(_gen: &mut rmcp::schemars::SchemaGenerator) -> rmcp::schemars::Schema {
        schema_from_json(json!({
            "type": "object",
            "properties": {
                "blocking": {
                    "type": "boolean",
                    "description": "True to raise shield (start blocking), false to lower shield (stop blocking)"
                }
            },
            "required": ["blocking"],
            "additionalProperties": false
        }))
    }
}

/// Handle `shield_block` MCP tool.
///
/// Checks online status, then sends [`BotCommand::ShieldBlock`].
/// The `blocking` parameter is included in the response metadata.
pub async fn handle_shield_block(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    input: ShieldBlockInput,
) -> String {
    if !state.is_online() {
        return r#"{"success":false,"error":"Bot is not connected to a server"}"#.to_string();
    }

    let cmd = BotCommand::ShieldBlock;
    match sender.send_command(cmd).await {
        Ok(result) => {
            let mut json = serde_json::to_value(&result).unwrap_or_default();
            if let Some(obj) = json.as_object_mut() {
                obj.insert("blocking".to_string(), Value::Bool(input.blocking));
            }
            serde_json::to_string(&json).unwrap_or_else(|e| {
                format!(r#"{{"success":false,"error":"Serialization error: {e}"}}"#)
            })
        }
        Err(e) => format!(r#"{{"success":false,"error":"{e}"}}"#),
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
        let (sender, _receiver) = create_command_channel(4);
        (state, sender)
    }

    fn make_online(state: &SharedState) {
        state.set_online(true);
    }

    /// Create a channel where the receiver echoes back a successful BotResult.
    fn make_echo_channel() -> (Arc<SharedState>, BotCommandSender) {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, mut receiver) = create_command_channel(4);

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
            },
            timestamp: 1,
            chunk_summary: vec![],
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
        assert!(result.contains("not connected"));
    }

    #[tokio::test]
    async fn test_attack_entity_not_found() {
        let (state, sender) = setup();
        make_online(&state);
        // No entities in default snapshot
        let input = AttackEntityInput { entity_id: 99 };
        let result = handle_attack_entity(&state, &sender, input).await;
        assert!(result.contains("not found"));
    }

    #[tokio::test]
    async fn test_attack_entity_valid() {
        let (state, sender) = setup();
        make_online(&state);
        add_test_entity(&state, 42);
        let input = AttackEntityInput { entity_id: 42 };
        let result = handle_attack_entity(&state, &sender, input).await;
        let _: Value = serde_json::from_str(&result).expect("valid JSON");
    }

    // ── shield_block ────────────────────────────────────────────

    #[tokio::test]
    async fn test_shield_block_offline() {
        let (state, sender) = setup();
        let input = ShieldBlockInput { blocking: true };
        let result = handle_shield_block(&state, &sender, input).await;
        assert!(result.contains("not connected"));
    }

    #[tokio::test]
    async fn test_shield_block_start() {
        let (state, sender) = make_echo_channel();
        make_online(&state);
        let input = ShieldBlockInput { blocking: true };
        let result = handle_shield_block(&state, &sender, input).await;
        let json: Value = serde_json::from_str(&result).expect("valid JSON");
        assert_eq!(json.get("blocking"), Some(&Value::Bool(true)));
    }

    #[tokio::test]
    async fn test_shield_block_stop() {
        let (state, sender) = make_echo_channel();
        make_online(&state);
        let input = ShieldBlockInput { blocking: false };
        let result = handle_shield_block(&state, &sender, input).await;
        let json: Value = serde_json::from_str(&result).expect("valid JSON");
        assert_eq!(json.get("blocking"), Some(&Value::Bool(false)));
    }
}
