//! MCP tools for bot movement (move_to, walk_direction, jump, teleport).
//!
//! Each tool validates parameters, checks online status, and dispatches a
//! [`BotCommand`] through the bot command channel.

use std::sync::Arc;

use serde::Deserialize;
use serde_json::Value;

use crate::channel::BotCommandSender;
use crate::command_validate::validate_block_pos;
use crate::error::BotError;
use crate::state::SharedState;
use crate::types::{BlockPos, BotCommand, Direction, GameMode};

// ── Helper ──────────────────────────────────────────────────────────────────

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
#[derive(Deserialize, Default, rmcp::schemars::JsonSchema)]
pub struct MoveToInput {
    /// X coordinate to move to.
    pub x: i32,
    /// Y coordinate to move to.
    pub y: i32,
    /// Z coordinate to move to.
    pub z: i32,
}

/// Handle `move_to` MCP tool.
///
/// Validates coordinates, checks online status, then sends
/// [`BotCommand::MoveTo`].
pub async fn handle_move_to(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    input: MoveToInput,
) -> Result<String, BotError> {
    if let Err(e) = validate_block_pos(&BlockPos::new(input.x, input.y, input.z)) {
        return Err(BotError::InvalidParams(e));
    }

    if !state.is_online() {
        return Err(BotError::Offline(
            "Bot is not connected to a server".to_string(),
        ));
    }

    let cmd = BotCommand::MoveTo(BlockPos::new(input.x, input.y, input.z));
    match sender.send_command(cmd).await {
        Ok(result) => serde_json::to_string(&result)
            .map_err(|e| BotError::Internal(format!("Serialization error: {e}"))),
        Err(e) => Err(e),
    }
}

// ── walk_direction ──────────────────────────────────────────────────────────

/// Input for the `walk_direction` MCP tool.
#[derive(Deserialize, Default, rmcp::schemars::JsonSchema)]
pub struct WalkDirectionInput {
    /// Cardinal direction to walk. One of: north, south, east, west, up, down,
    /// northeast, northwest, southeast, southwest.
    pub direction: String,
    /// Number of blocks to walk in the given direction (1-1000).
    #[schemars(range(min = 1, max = 1000))]
    pub distance: u32,
}

/// Handle `walk_direction` MCP tool.
///
/// Parses the direction string, validates distance > 0, checks online status,
/// then sends [`BotCommand::WalkDirection`].
pub async fn handle_walk_direction(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    input: WalkDirectionInput,
) -> Result<String, BotError> {
    let direction = match parse_direction(&input.direction) {
        Some(d) => d,
        None => {
            return Err(BotError::InvalidParams(format!(
                "Invalid direction: '{}'. Must be one of: north, south, east, west, up, down, northeast, northwest, southeast, southwest",
                input.direction
            )));
        }
    };

    if input.distance < 1 || input.distance > 1000 {
        return Err(BotError::InvalidParams(format!(
            "Distance must be between 1 and 1000, got {}",
            input.distance
        )));
    }

    if !state.is_online() {
        return Err(BotError::Offline(
            "Bot is not connected to a server".to_string(),
        ));
    }

    let cmd = BotCommand::WalkDirection(direction, input.distance);
    match sender.send_command(cmd).await {
        Ok(result) => {
            let mut json = serde_json::to_value(&result)
                .map_err(|e| BotError::Internal(format!("Serialization error: {e}")))?;
            if let Some(obj) = json.as_object_mut() {
                obj.insert("distance".to_string(), Value::Number(input.distance.into()));
            }
            serde_json::to_string(&json)
                .map_err(|e| BotError::Internal(format!("Serialization error: {e}")))
        }
        Err(e) => Err(e),
    }
}

// ── jump ────────────────────────────────────────────────────────────────────

/// Input for the `jump` MCP tool (no parameters needed).
#[derive(Deserialize, Default, rmcp::schemars::JsonSchema)]
pub struct JumpInput {}

/// Handle `jump` MCP tool.
///
/// Checks online status, then sends [`BotCommand::Jump`].
pub async fn handle_jump(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    _input: JumpInput,
) -> Result<String, BotError> {
    if !state.is_online() {
        return Err(BotError::Offline(
            "Bot is not connected to a server".to_string(),
        ));
    }

    let cmd = BotCommand::Jump;
    match sender.send_command(cmd).await {
        Ok(result) => serde_json::to_string(&result)
            .map_err(|e| BotError::Internal(format!("Serialization error: {e}"))),
        Err(e) => Err(e),
    }
}

// ── teleport ────────────────────────────────────────────────────────────────

/// Input for the `teleport` MCP tool.
#[derive(Deserialize, Default, rmcp::schemars::JsonSchema)]
pub struct TeleportInput {
    /// X coordinate to teleport to.
    pub x: i32,
    /// Y coordinate to teleport to.
    pub y: i32,
    /// Z coordinate to teleport to.
    pub z: i32,
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
) -> Result<String, BotError> {
    if let Err(e) = validate_block_pos(&BlockPos::new(input.x, input.y, input.z)) {
        return Err(BotError::InvalidParams(e));
    }

    if !state.is_online() {
        return Err(BotError::Offline(
            "Bot is not connected to a server".to_string(),
        ));
    }

    // Teleport requires Creative mode (or operator permissions)
    {
        let snap = state.read_snapshot();
        if snap.self_player.gamemode != GameMode::Creative {
            return Err(BotError::PermissionDenied(
                "Teleport requires Creative mode".to_string(),
            ));
        }
    }

    let cmd = BotCommand::Teleport(BlockPos::new(input.x, input.y, input.z));
    match sender.send_command(cmd).await {
        Ok(result) => serde_json::to_string(&result)
            .map_err(|e| BotError::Internal(format!("Serialization error: {e}"))),
        Err(e) => Err(e),
    }
}

// ── smart_move ──────────────────────────────────────────────────────────────

/// Input for the `smart_move` MCP tool.
#[derive(Deserialize, Default, rmcp::schemars::JsonSchema)]
pub struct SmartMoveInput {
    /// X coordinate to move to.
    pub x: i32,
    /// Y coordinate to move to.
    pub y: i32,
    /// Z coordinate to move to.
    pub z: i32,
}

/// Handle `smart_move` MCP tool.
///
/// Smart movement toward a target coordinate. The bot auto-jumps over
/// 1-block obstacles and stops when encountering an impassable obstacle
/// (2+ blocks high). Validates coordinates, checks online status, then
/// sends [`BotCommand::SmartMove`].
pub async fn handle_smart_move(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    input: SmartMoveInput,
) -> Result<String, BotError> {
    if let Err(e) = validate_block_pos(&BlockPos::new(input.x, input.y, input.z)) {
        return Err(BotError::InvalidParams(e));
    }

    if !state.is_online() {
        return Err(BotError::Offline(
            "Bot is not connected to a server".to_string(),
        ));
    }

    let cmd = BotCommand::SmartMove(BlockPos::new(input.x, input.y, input.z));
    match sender.send_command(cmd).await {
        Ok(result) => serde_json::to_string(&result)
            .map_err(|e| BotError::Internal(format!("Serialization error: {e}"))),
        Err(e) => Err(e),
    }
}

// ── fly_to ──────────────────────────────────────────────────────────────────

/// Input for the `fly_to` MCP tool.
#[derive(Deserialize, Default, rmcp::schemars::JsonSchema)]
pub struct FlyToInput {
    /// X coordinate to fly to.
    pub x: i32,
    /// Y coordinate to fly to.
    pub y: i32,
    /// Z coordinate to fly to.
    pub z: i32,
}

/// Handle `fly_to` MCP tool.
///
/// Creative mode only. Flies toward a target coordinate in 3D. Stops on
/// obstacle. Fails if not in Creative mode. Validates coordinates, checks
/// online status and gamemode, then sends [`BotCommand::FlyTo`].
pub async fn handle_fly_to(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    input: FlyToInput,
) -> Result<String, BotError> {
    if let Err(e) = validate_block_pos(&BlockPos::new(input.x, input.y, input.z)) {
        return Err(BotError::InvalidParams(e));
    }

    if !state.is_online() {
        return Err(BotError::Offline(
            "Bot is not connected to a server".to_string(),
        ));
    }

    // Fly requires Creative mode
    {
        let snap = state.read_snapshot();
        if snap.self_player.gamemode != GameMode::Creative {
            return Err(BotError::PermissionDenied(
                "Fly requires Creative mode".to_string(),
            ));
        }
    }

    let cmd = BotCommand::FlyTo(BlockPos::new(input.x, input.y, input.z));
    match sender.send_command(cmd).await {
        Ok(result) => serde_json::to_string(&result)
            .map_err(|e| BotError::Internal(format!("Serialization error: {e}"))),
        Err(e) => Err(e),
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
        let (sender, mut receiver) = create_command_channel(4, Arc::clone(&state));

        // Spawn a mock receiver so tests that actually send commands get a
        // successful response instead of a "channel closed" error.
        tokio::spawn(async move {
            use crate::types::BotResult;
            while let Some(cmd) = receiver.recv().await {
                let result = Ok(BotResult {
                    success: true,
                    message: "ok".into(),
                    data: None,
                });
                let _ = cmd.respond_to.send(result);
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
                inventory: Vec::new(),
                position_precise: None,
                yaw: None,
            },
            timestamp: 1,
            chunk_summary: vec![],
            commands_enabled: None,
            ..Default::default()
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
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    #[tokio::test]
    async fn test_move_to_invalid_coords() {
        let (state, sender) = setup();
        make_online(&state);
        let input = MoveToInput { x: 0, y: 500, z: 0 };
        let result = handle_move_to(&state, &sender, input).await;
        assert!(
            matches!(result, Err(BotError::InvalidParams(ref msg)) if msg.contains("out of bounds") || msg.contains("out of range"))
        );
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
        let result = handle_move_to(&state, &sender, input).await.unwrap();
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
        assert!(matches!(result, Err(BotError::Offline(_))));
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
        assert!(matches!(result, Err(BotError::InvalidParams(_))));
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
        assert!(matches!(result, Err(BotError::InvalidParams(_))));
    }

    #[tokio::test]
    async fn test_walk_direction_distance_too_large() {
        let (state, sender) = setup();
        make_online(&state);
        let input = WalkDirectionInput {
            direction: "north".into(),
            distance: 1001,
        };
        let result = handle_walk_direction(&state, &sender, input).await;
        assert!(
            matches!(result, Err(BotError::InvalidParams(ref msg))
                if msg.contains("1 and 1000")),
            "expected distance-too-large error, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_walk_direction_max_distance_valid() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        make_online(&state);
        let (sender, mut receiver) = create_command_channel(4, Arc::clone(&state));

        let responder = tokio::spawn(async move {
            let wrapped = receiver.recv().await.expect("should receive command");
            assert!(
                matches!(
                    wrapped.command,
                    BotCommand::WalkDirection(Direction::South, 1000)
                ),
                "expected WalkDirection(South, 1000) at max distance, got: {:?}",
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

        let input = WalkDirectionInput {
            direction: "south".into(),
            distance: 1000,
        };
        let result = handle_walk_direction(&state, &sender, input).await.unwrap();
        let json: Value = serde_json::from_str(&result).expect("valid JSON");
        assert_eq!(json.get("distance"), Some(&Value::Number(1000.into())));
        responder.await.expect("responder should finish");
    }

    #[tokio::test]
    async fn test_walk_direction_valid() {
        // Verify the parsed direction and distance are propagated as
        // BotCommand::WalkDirection(Direction::North, 3).
        let state = Arc::new(SharedState::new(AppConfig::default()));
        make_online(&state);
        let (sender, mut receiver) = create_command_channel(4, Arc::clone(&state));

        let responder = tokio::spawn(async move {
            let wrapped = receiver.recv().await.expect("should receive command");
            assert!(
                matches!(
                    wrapped.command,
                    BotCommand::WalkDirection(Direction::North, 3)
                ),
                "expected WalkDirection(North, 3), got: {:?}",
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

        let input = WalkDirectionInput {
            direction: "north".into(),
            distance: 3,
        };
        let result = handle_walk_direction(&state, &sender, input).await.unwrap();
        let json: Value = serde_json::from_str(&result).expect("valid JSON");
        assert!(
            json.get("success")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            "expected success, got: {result}"
        );
        // The response metadata should also carry the distance.
        assert_eq!(json.get("distance"), Some(&Value::Number(3.into())));

        responder.await.expect("responder should finish");
    }

    // ── jump ────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_jump_offline() {
        let (state, sender) = setup();
        let input = JumpInput {};
        let result = handle_jump(&state, &sender, input).await;
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    #[tokio::test]
    async fn test_jump_valid() {
        let (state, sender) = setup();
        make_online(&state);
        let input = JumpInput {};
        let result = handle_jump(&state, &sender, input).await.unwrap();
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
        assert!(matches!(result, Err(BotError::Offline(_))));
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
        assert!(matches!(result, Err(BotError::PermissionDenied(_))));
    }

    #[tokio::test]
    async fn test_teleport_invalid_coords() {
        let (state, sender) = setup();
        make_online(&state);
        make_creative(&state);
        let input = TeleportInput { x: 0, y: 500, z: 0 };
        let result = handle_teleport(&state, &sender, input).await;
        assert!(
            matches!(result, Err(BotError::InvalidParams(ref msg)) if msg.contains("out of bounds") || msg.contains("out of range"))
        );
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
        let result = handle_teleport(&state, &sender, input).await.unwrap();
        let _: Value = serde_json::from_str(&result).expect("valid JSON");
    }

    // ── smart_move ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_smart_move_offline() {
        let (state, sender) = setup();
        let input = SmartMoveInput { x: 0, y: 64, z: 0 };
        let result = handle_smart_move(&state, &sender, input).await;
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    #[tokio::test]
    async fn test_smart_move_invalid_coords() {
        let (state, sender) = setup();
        make_online(&state);
        let input = SmartMoveInput { x: 0, y: 500, z: 0 };
        let result = handle_smart_move(&state, &sender, input).await;
        assert!(
            matches!(result, Err(BotError::InvalidParams(ref msg)) if msg.contains("out of bounds") || msg.contains("out of range"))
        );
    }

    #[tokio::test]
    async fn test_smart_move_valid() {
        let (state, sender) = setup();
        make_online(&state);
        let input = SmartMoveInput {
            x: 10,
            y: 64,
            z: -5,
        };
        let result = handle_smart_move(&state, &sender, input).await.unwrap();
        let _: Value = serde_json::from_str(&result).expect("valid JSON");
    }

    #[tokio::test]
    async fn test_smart_move_sends_correct_command() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        make_online(&state);
        let (sender, mut receiver) = create_command_channel(4, Arc::clone(&state));

        let responder = tokio::spawn(async move {
            let wrapped = receiver.recv().await.expect("should receive command");
            assert!(
                matches!(
                    wrapped.command,
                    BotCommand::SmartMove(pos) if pos == BlockPos::new(7, 64, -3)
                ),
                "expected SmartMove((7, 64, -3)), got: {:?}",
                wrapped.command
            );
            wrapped
                .respond_to
                .send(Ok(crate::types::BotResult {
                    success: true,
                    message: "reached".into(),
                    data: None,
                }))
                .expect("should respond");
        });

        let input = SmartMoveInput { x: 7, y: 64, z: -3 };
        let result = handle_smart_move(&state, &sender, input).await.unwrap();
        let json: Value = serde_json::from_str(&result).expect("valid JSON");
        assert!(
            json.get("success")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        );
        responder.await.expect("responder should finish");
    }

    // ── fly_to ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_fly_to_offline() {
        let (state, sender) = setup();
        make_creative(&state);
        let input = FlyToInput { x: 0, y: 80, z: 0 };
        let result = handle_fly_to(&state, &sender, input).await;
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    #[tokio::test]
    async fn test_fly_to_not_creative() {
        let (state, sender) = setup();
        make_online(&state);
        // Default snapshot is Survival mode
        let input = FlyToInput { x: 0, y: 80, z: 0 };
        let result = handle_fly_to(&state, &sender, input).await;
        assert!(matches!(result, Err(BotError::PermissionDenied(_))));
    }

    #[tokio::test]
    async fn test_fly_to_invalid_coords() {
        let (state, sender) = setup();
        make_online(&state);
        make_creative(&state);
        let input = FlyToInput { x: 0, y: 500, z: 0 };
        let result = handle_fly_to(&state, &sender, input).await;
        assert!(
            matches!(result, Err(BotError::InvalidParams(ref msg)) if msg.contains("out of bounds") || msg.contains("out of range"))
        );
    }

    #[tokio::test]
    async fn test_fly_to_valid() {
        let (state, sender) = setup();
        make_online(&state);
        make_creative(&state);
        let input = FlyToInput {
            x: 100,
            y: 80,
            z: 200,
        };
        let result = handle_fly_to(&state, &sender, input).await.unwrap();
        let _: Value = serde_json::from_str(&result).expect("valid JSON");
    }

    #[tokio::test]
    async fn test_fly_to_sends_correct_command() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        make_online(&state);
        make_creative(&state);
        let (sender, mut receiver) = create_command_channel(4, Arc::clone(&state));

        let responder = tokio::spawn(async move {
            let wrapped = receiver.recv().await.expect("should receive command");
            assert!(
                matches!(
                    wrapped.command,
                    BotCommand::FlyTo(pos) if pos == BlockPos::new(12, 80, 5)
                ),
                "expected FlyTo((12, 80, 5)), got: {:?}",
                wrapped.command
            );
            wrapped
                .respond_to
                .send(Ok(crate::types::BotResult {
                    success: true,
                    message: "flew".into(),
                    data: None,
                }))
                .expect("should respond");
        });

        let input = FlyToInput { x: 12, y: 80, z: 5 };
        let result = handle_fly_to(&state, &sender, input).await.unwrap();
        let json: Value = serde_json::from_str(&result).expect("valid JSON");
        assert!(
            json.get("success")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        );
        responder.await.expect("responder should finish");
    }
}
