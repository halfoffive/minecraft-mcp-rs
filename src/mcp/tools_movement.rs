//! MCP tools for bot movement (move_to, walk_direction, jump, teleport).
//!
//! Each tool validates parameters, checks online status, and dispatches a
//! [`BotCommand`](crate::types::BotCommand) through the bot command channel.
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
use crate::command_validate::validate_block_pos;
use crate::state::SharedState;
use crate::types::{BlockPos, BotCommand, Direction, GameMode};

// ── Helper ──────────────────────────────────────────────────────────────────

fn schema_from_json(v: Value) -> rmcp::schemars::Schema {
    let map: Map<String, Value> = v.as_object().cloned().unwrap_or_default();
    rmcp::schemars::Schema::from(map)
}

/// Parse a direction string (case-insensitive) into a [`Direction`].
fn parse_direction(s: &str) -> Option<Direction> {
    match s.to_lowercase().as_str() {
        "north" => Some(Direction::North),
        "south" => Some(Direction::South),
        "east" => Some(Direction::East),
        "west" => Some(Direction::West),
        "up" => Some(Direction::Up),
        "down" => Some(Direction::Down),
        "northeast" | "north-east" | "north_east" => Some(Direction::NorthEast),
        "northwest" | "north-west" | "north_west" => Some(Direction::NorthWest),
        "southeast" | "south-east" | "south_east" => Some(Direction::SouthEast),
        "southwest" | "south-west" | "south_west" => Some(Direction::SouthWest),
        _ => None,
    }
}

// ── move_to ─────────────────────────────────────────────────────────────────

/// Input for the `move_to` MCP tool.
#[derive(Deserialize, Default)]
pub struct MoveToInput {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl rmcp::schemars::JsonSchema for MoveToInput {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("MoveToInput")
    }

    fn json_schema(_gen: &mut rmcp::schemars::SchemaGenerator) -> rmcp::schemars::Schema {
        schema_from_json(json!({
            "type": "object",
            "properties": {
                "x": {
                    "type": "integer",
                    "description": "X coordinate to move to"
                },
                "y": {
                    "type": "integer",
                    "description": "Y coordinate to move to"
                },
                "z": {
                    "type": "integer",
                    "description": "Z coordinate to move to"
                }
            },
            "required": ["x", "y", "z"],
            "additionalProperties": false
        }))
    }
}

/// Handle `move_to` MCP tool.
///
/// Validates coordinates, checks online status, then sends
/// [`BotCommand::MoveTo`].
pub async fn handle_move_to(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    input: MoveToInput,
) -> String {
    if let Err(e) = validate_block_pos(&BlockPos::new(input.x, input.y, input.z)) {
        return format!(r#"{{"success":false,"error":"{e}"}}"#);
    }

    if !state.is_online() {
        return r#"{"success":false,"error":"Bot is not connected to a server"}"#.to_string();
    }

    let cmd = BotCommand::MoveTo(BlockPos::new(input.x, input.y, input.z));
    match sender.send_command(cmd).await {
        Ok(result) => serde_json::to_string(&result).unwrap_or_else(|e| {
            format!(r#"{{"success":false,"error":"Serialization error: {e}"}}"#)
        }),
        Err(e) => format!(r#"{{"success":false,"error":"{e}"}}"#),
    }
}

// ── walk_direction ──────────────────────────────────────────────────────────

/// Input for the `walk_direction` MCP tool.
#[derive(Deserialize, Default)]
pub struct WalkDirectionInput {
    pub direction: String,
    pub distance: u32,
}

impl rmcp::schemars::JsonSchema for WalkDirectionInput {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("WalkDirectionInput")
    }

    fn json_schema(_gen: &mut rmcp::schemars::SchemaGenerator) -> rmcp::schemars::Schema {
        schema_from_json(json!({
            "type": "object",
            "properties": {
                "direction": {
                    "type": "string",
                    "description": "Cardinal direction to walk. One of: north, south, east, west, up, down, northeast, northwest, southeast, southwest",
                    "enum": ["north", "south", "east", "west", "up", "down", "northeast", "northwest", "southeast", "southwest"]
                },
                "distance": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Number of blocks to walk in the given direction"
                }
            },
            "required": ["direction", "distance"],
            "additionalProperties": false
        }))
    }
}

/// Handle `walk_direction` MCP tool.
///
/// Parses the direction string, validates distance > 0, checks online status,
/// then sends [`BotCommand::WalkDirection`].
pub async fn handle_walk_direction(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    input: WalkDirectionInput,
) -> String {
    let direction = match parse_direction(&input.direction) {
        Some(d) => d,
        None => {
            return format!(
                r#"{{"success":false,"error":"Invalid direction: '{}'. Must be one of: north, south, east, west, up, down, northeast, northwest, southeast, southwest"}}"#,
                input.direction
            );
        }
    };

    if input.distance == 0 {
        return r#"{"success":false,"error":"Distance must be greater than 0"}"#.to_string();
    }

    if !state.is_online() {
        return r#"{"success":false,"error":"Bot is not connected to a server"}"#.to_string();
    }

    let cmd = BotCommand::WalkDirection(direction);
    match sender.send_command(cmd).await {
        Ok(result) => {
            let mut json = serde_json::to_value(&result).unwrap_or_default();
            if let Some(obj) = json.as_object_mut() {
                obj.insert("distance".to_string(), Value::Number(input.distance.into()));
            }
            serde_json::to_string(&json).unwrap_or_else(|e| {
                format!(r#"{{"success":false,"error":"Serialization error: {e}"}}"#)
            })
        }
        Err(e) => format!(r#"{{"success":false,"error":"{e}"}}"#),
    }
}

// ── jump ────────────────────────────────────────────────────────────────────

/// Input for the `jump` MCP tool (no parameters needed).
#[derive(Deserialize, Default)]
pub struct JumpInput {}

impl rmcp::schemars::JsonSchema for JumpInput {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("JumpInput")
    }

    fn json_schema(_gen: &mut rmcp::schemars::SchemaGenerator) -> rmcp::schemars::Schema {
        schema_from_json(json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        }))
    }
}

/// Handle `jump` MCP tool.
///
/// Checks online status, then sends [`BotCommand::Jump`].
pub async fn handle_jump(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    _input: JumpInput,
) -> String {
    if !state.is_online() {
        return r#"{"success":false,"error":"Bot is not connected to a server"}"#.to_string();
    }

    let cmd = BotCommand::Jump;
    match sender.send_command(cmd).await {
        Ok(result) => serde_json::to_string(&result).unwrap_or_else(|e| {
            format!(r#"{{"success":false,"error":"Serialization error: {e}"}}"#)
        }),
        Err(e) => format!(r#"{{"success":false,"error":"{e}"}}"#),
    }
}

// ── teleport ────────────────────────────────────────────────────────────────

/// Input for the `teleport` MCP tool.
#[derive(Deserialize, Default)]
pub struct TeleportInput {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl rmcp::schemars::JsonSchema for TeleportInput {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("TeleportInput")
    }

    fn json_schema(_gen: &mut rmcp::schemars::SchemaGenerator) -> rmcp::schemars::Schema {
        schema_from_json(json!({
            "type": "object",
            "properties": {
                "x": {
                    "type": "integer",
                    "description": "X coordinate to teleport to"
                },
                "y": {
                    "type": "integer",
                    "description": "Y coordinate to teleport to"
                },
                "z": {
                    "type": "integer",
                    "description": "Z coordinate to teleport to"
                }
            },
            "required": ["x", "y", "z"],
            "additionalProperties": false
        }))
    }
}

/// Handle `teleport` MCP tool.
///
/// Validates coordinates, requires player to be in Creative mode
/// (teleport is an operator-level command), checks online status,
/// then sends [`BotCommand::Teleport`].
pub async fn handle_teleport(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    input: TeleportInput,
) -> String {
    if let Err(e) = validate_block_pos(&BlockPos::new(input.x, input.y, input.z)) {
        return format!(r#"{{"success":false,"error":"{e}"}}"#);
    }

    // Teleport requires Creative mode (or operator permissions)
    {
        let snap = state.read_snapshot();
        if snap.self_player.gamemode != GameMode::Creative {
            return r#"{"success":false,"error":"Teleport requires Creative mode"}"#.to_string();
        }
    }

    if !state.is_online() {
        return r#"{"success":false,"error":"Bot is not connected to a server"}"#.to_string();
    }

    let cmd = BotCommand::Teleport(BlockPos::new(input.x, input.y, input.z));
    match sender.send_command(cmd).await {
        Ok(result) => serde_json::to_string(&result).unwrap_or_else(|e| {
            format!(r#"{{"success":false,"error":"Serialization error: {e}"}}"#)
        }),
        Err(e) => format!(r#"{{"success":false,"error":"{e}"}}"#),
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::create_command_channel;
    use crate::config::AppConfig;
    use crate::types::{SelfPlayer, WorldSnapshot};

    fn setup() -> (Arc<SharedState>, BotCommandSender) {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, _receiver) = create_command_channel(4);
        (state, sender)
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

    fn make_online(state: &SharedState) {
        state.set_online(true);
    }

    fn make_creative(state: &SharedState) {
        let snap = WorldSnapshot {
            blocks: vec![],
            entities: vec![],
            self_player: SelfPlayer {
                uuid: "test".into(),
                username: "TestBot".into(),
                position: BlockPos::new(0, 64, 0),
                health: 20.0,
                hunger: 20,
                gamemode: GameMode::Creative,
                held_item_slot: 0,
            },
            timestamp: 1,
            chunk_summary: vec![],
        };
        state.update_snapshot(snap);
    }

    // ── parse_direction ──────────────────────────────────────────

    #[test]
    fn test_parse_direction_north() {
        assert_eq!(parse_direction("north"), Some(Direction::North));
    }

    #[test]
    fn test_parse_direction_case_insensitive() {
        assert_eq!(parse_direction("NORTH"), Some(Direction::North));
        assert_eq!(parse_direction("South"), Some(Direction::South));
    }

    #[test]
    fn test_parse_direction_all_variants() {
        for dir in ["north", "south", "east", "west", "up", "down"] {
            assert!(
                parse_direction(dir).is_some(),
                "direction '{dir}' should parse"
            );
        }
    }

    #[test]
    fn test_parse_direction_diagonals() {
        assert_eq!(parse_direction("northeast"), Some(Direction::NorthEast));
        assert_eq!(parse_direction("north-east"), Some(Direction::NorthEast));
        assert_eq!(parse_direction("north_east"), Some(Direction::NorthEast));
    }

    #[test]
    fn test_parse_direction_invalid() {
        assert_eq!(parse_direction("left"), None);
        assert_eq!(parse_direction(""), None);
    }

    // ── move_to ─────────────────────────────────────────────────

    #[tokio::test]
    async fn test_move_to_offline() {
        let (state, sender) = setup();
        let input = MoveToInput { x: 0, y: 64, z: 0 };
        let result = handle_move_to(&state, &sender, input).await;
        assert!(result.contains("not connected"));
    }

    #[tokio::test]
    async fn test_move_to_invalid_coords() {
        let (state, sender) = setup();
        make_online(&state);
        let input = MoveToInput { x: 0, y: 500, z: 0 };
        let result = handle_move_to(&state, &sender, input).await;
        assert!(result.contains("out of bounds") || result.contains("out of range"));
    }

    #[tokio::test]
    async fn test_move_to_valid() {
        let (state, sender) = setup();
        make_online(&state);
        let input = MoveToInput {
            x: 10,
            y: 64,
            z: -5,
        };
        let result = handle_move_to(&state, &sender, input).await;
        let _: Value = serde_json::from_str(&result).expect("valid JSON");
    }

    // ── walk_direction ──────────────────────────────────────────

    #[tokio::test]
    async fn test_walk_direction_offline() {
        let (state, sender) = setup();
        let input = WalkDirectionInput {
            direction: "north".into(),
            distance: 1,
        };
        let result = handle_walk_direction(&state, &sender, input).await;
        assert!(result.contains("not connected"));
    }

    #[tokio::test]
    async fn test_walk_direction_invalid_direction() {
        let (state, sender) = setup();
        make_online(&state);
        let input = WalkDirectionInput {
            direction: "left".into(),
            distance: 1,
        };
        let result = handle_walk_direction(&state, &sender, input).await;
        assert!(result.contains("Invalid direction"));
    }

    #[tokio::test]
    async fn test_walk_direction_zero_distance() {
        let (state, sender) = setup();
        make_online(&state);
        let input = WalkDirectionInput {
            direction: "north".into(),
            distance: 0,
        };
        let result = handle_walk_direction(&state, &sender, input).await;
        assert!(result.contains("greater than 0"));
    }

    #[tokio::test]
    async fn test_walk_direction_valid() {
        let (state, sender) = make_echo_channel();
        make_online(&state);
        let input = WalkDirectionInput {
            direction: "north".into(),
            distance: 3,
        };
        let result = handle_walk_direction(&state, &sender, input).await;
        let json: Value = serde_json::from_str(&result).expect("valid JSON");
        assert!(
            json.get("success")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        );
    }

    // ── jump ────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_jump_offline() {
        let (state, sender) = setup();
        let input = JumpInput {};
        let result = handle_jump(&state, &sender, input).await;
        assert!(result.contains("not connected"));
    }

    #[tokio::test]
    async fn test_jump_valid() {
        let (state, sender) = setup();
        make_online(&state);
        let input = JumpInput {};
        let result = handle_jump(&state, &sender, input).await;
        let _: Value = serde_json::from_str(&result).expect("valid JSON");
    }

    // ── teleport ────────────────────────────────────────────────

    #[tokio::test]
    async fn test_teleport_offline() {
        let (state, sender) = setup();
        make_creative(&state);
        let input = TeleportInput {
            x: 100,
            y: 64,
            z: 200,
        };
        let result = handle_teleport(&state, &sender, input).await;
        assert!(result.contains("not connected"));
    }

    #[tokio::test]
    async fn test_teleport_not_creative() {
        let (state, sender) = setup();
        make_online(&state);
        // Default snapshot is Survival mode
        let input = TeleportInput {
            x: 100,
            y: 64,
            z: 200,
        };
        let result = handle_teleport(&state, &sender, input).await;
        assert!(result.contains("requires Creative"));
    }

    #[tokio::test]
    async fn test_teleport_invalid_coords() {
        let (state, sender) = setup();
        make_online(&state);
        make_creative(&state);
        let input = TeleportInput { x: 0, y: 500, z: 0 };
        let result = handle_teleport(&state, &sender, input).await;
        assert!(result.contains("out of bounds") || result.contains("out of range"));
    }

    #[tokio::test]
    async fn test_teleport_valid() {
        let (state, sender) = setup();
        make_online(&state);
        make_creative(&state);
        let input = TeleportInput {
            x: 100,
            y: 64,
            z: 200,
        };
        let result = handle_teleport(&state, &sender, input).await;
        let _: Value = serde_json::from_str(&result).expect("valid JSON");
    }
}
