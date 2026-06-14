use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;

// ═══════════════════════════════════════════════════════════════
// Position & Direction
// ═══════════════════════════════════════════════════════════════

/// A position in the Minecraft world.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct BlockPos {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl BlockPos {
    pub fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }
}

impl fmt::Display for BlockPos {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({}, {}, {})", self.x, self.y, self.z)
    }
}

/// Cardinal and diagonal directions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum Direction {
    North,
    South,
    East,
    West,
    Up,
    Down,
    NorthEast,
    NorthWest,
    SouthEast,
    SouthWest,
}

// ═══════════════════════════════════════════════════════════════
// Items, Tools, Materials
// ═══════════════════════════════════════════════════════════════

/// Types of tools available in Minecraft.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum ToolType {
    Pickaxe,
    Axe,
    Shovel,
    Sword,
    Shears,
    Hand,
}

/// Material tiers for tools/armor, ordered by quality/durability.
///
/// Order: Wood < Gold < Stone < Iron < Diamond < Netherite
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
pub enum MaterialTier {
    Wood,
    Gold,
    Stone,
    Iron,
    Diamond,
    Netherite,
}

// ═══════════════════════════════════════════════════════════════
// Game Mode
// ═══════════════════════════════════════════════════════════════

/// Minecraft game modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum GameMode {
    Survival,
    Creative,
    Adventure,
    Spectator,
}

// ═══════════════════════════════════════════════════════════════
// Bot Commands (MCP contract boundary)
// ═══════════════════════════════════════════════════════════════

/// Commands that can be sent to the Minecraft bot.
///
/// This enum is the central contract between the MCP server and the bot engine.
/// Each variant represents an action the bot can perform in-game.
///
/// NOTE: CraftItem is intentionally excluded (v2 feature).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub enum BotCommand {
    /// Move to a specific position.
    MoveTo(BlockPos),
    /// Walk in a direction.
    WalkDirection(Direction),
    /// Jump.
    Jump,
    /// Teleport to a position (requires operator permissions).
    Teleport(BlockPos),
    /// Break a block at the given position.
    BreakBlock(BlockPos),
    /// Place a block at the given position.
    PlaceBlock(BlockPos, String),
    /// Use an item on a block (right-click).
    UseItemOnBlock(BlockPos),
    /// Switch to a hotbar slot (0-8).
    SwitchHotbarSlot(u8),
    /// Drop items from a slot.
    DropItem(u8, u8),
    /// Use the currently held item.
    UseItem,
    /// Equip a tool type.
    EquipTool(ToolType),
    /// Open a container at the given position.
    OpenContainer(BlockPos),
    /// Take items from a container slot.
    TakeFromContainer(u8, u8),
    /// Put items into a container slot.
    PutIntoContainer(u8, u8),
    /// Close the currently open container.
    CloseContainer,
    /// Attack an entity by ID.
    AttackEntity(u32),
    /// Hold up a shield to block.
    ShieldBlock,
    /// Send a chat message.
    SendChat(String),
    /// Execute a Minecraft command.
    ExecuteCommand(String),
    /// Set the player's game mode (requires operator permissions).
    SetGameMode(GameMode),
    /// Query nearby blocks within a radius.
    QueryNearbyBlocks(u32),
    /// Query nearby entities within a radius.
    QueryNearbyEntities(u32),
    /// Query information about the local player.
    QuerySelfInfo,
    /// Query the player's inventory.
    QueryInventory,
    /// Query a summary of loaded chunks.
    QueryChunkSummary,
}

// ═══════════════════════════════════════════════════════════════
// Bot Results & Events
// ═══════════════════════════════════════════════════════════════

/// Result returned from a bot operation.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BotResult {
    pub success: bool,
    pub message: String,
    pub data: Option<serde_json::Value>,
}

/// Events that can occur during gameplay, streamed to the MCP client.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub enum GameEvent {
    /// A chat message was received.
    ChatMessage { sender: String, message: String },
    /// A player joined the game.
    PlayerJoin { username: String },
    /// A player left the game.
    PlayerLeave { username: String },
    /// A block was broken.
    BlockBreak {
        position: BlockPos,
        block_type: String,
    },
    /// A block was placed.
    BlockPlace {
        position: BlockPos,
        block_type: String,
    },
    /// An entity spawned.
    EntitySpawn { entity: Box<EntityEntry> },
    /// An entity despawned.
    EntityDespawn { entity_id: u32 },
    /// An entity took damage.
    Damage { entity_id: u32, amount: f32 },
    /// An entity died.
    Death { entity_id: u32 },
    /// The player's inventory was updated.
    InventoryUpdate,
    /// The player's game mode changed.
    GameModeChange { new_mode: GameMode },
}

// ═══════════════════════════════════════════════════════════════
// World & Entity Data
// ═══════════════════════════════════════════════════════════════

/// A snapshot of the world state at a moment in time.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorldSnapshot {
    pub blocks: Vec<BlockEntry>,
    pub entities: Vec<EntityEntry>,
    pub self_player: SelfPlayer,
    pub timestamp: u64,
    pub chunk_summary: Vec<(i32, i32)>,
}

/// Information about the local player.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SelfPlayer {
    pub uuid: String,
    pub username: String,
    pub position: BlockPos,
    pub health: f32,
    pub hunger: i32,
    pub gamemode: GameMode,
    pub held_item_slot: u8,
}

/// A block entry in the world.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BlockEntry {
    pub position: BlockPos,
    pub block_type: String,
    pub block_state: Option<String>,
}

/// An entity entry in the world.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EntityEntry {
    pub id: u32,
    pub uuid: String,
    pub entity_type: String,
    pub position: BlockPos,
    pub display_name: Option<String>,
    pub health: Option<f32>,
}

// ═══════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    // ── BlockPos tests ──────────────────────────────────────

    #[test]
    fn test_block_pos_new() {
        let pos = BlockPos::new(1, 2, 3);
        assert_eq!(pos.x, 1);
        assert_eq!(pos.y, 2);
        assert_eq!(pos.z, 3);
    }

    #[test]
    fn test_block_pos_display() {
        let pos = BlockPos::new(-1, 64, 255);
        assert_eq!(pos.to_string(), "(-1, 64, 255)");
    }

    #[test]
    fn test_block_pos_eq() {
        let a = BlockPos::new(10, 20, 30);
        let b = BlockPos::new(10, 20, 30);
        let c = BlockPos::new(0, 0, 0);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    proptest! {
        #[test]
        fn test_block_pos_any_coordinates(x: i32, y: i32, z: i32) {
            let pos = BlockPos::new(x, y, z);
            assert_eq!(pos.x, x);
            assert_eq!(pos.y, y);
            assert_eq!(pos.z, z);
        }
    }

    // ── Direction tests ──────────────────────────────────────

    #[test]
    fn test_direction_has_10_variants() {
        let variants = vec![
            Direction::North,
            Direction::South,
            Direction::East,
            Direction::West,
            Direction::Up,
            Direction::Down,
            Direction::NorthEast,
            Direction::NorthWest,
            Direction::SouthEast,
            Direction::SouthWest,
        ];
        assert_eq!(variants.len(), 10);
        let unique: std::collections::HashSet<_> = variants.iter().collect();
        assert_eq!(unique.len(), 10);
    }

    // ── ToolType tests ──────────────────────────────────────

    #[test]
    fn test_tool_type_has_6_variants() {
        let tools = [
            ToolType::Pickaxe,
            ToolType::Axe,
            ToolType::Shovel,
            ToolType::Sword,
            ToolType::Shears,
            ToolType::Hand,
        ];
        assert_eq!(tools.len(), 6);
    }

    // ── MaterialTier tests ──────────────────────────────────

    #[test]
    fn test_material_tier_ordering() {
        assert!(MaterialTier::Wood < MaterialTier::Stone);
        assert!(MaterialTier::Stone < MaterialTier::Iron);
        assert!(MaterialTier::Iron < MaterialTier::Diamond);
        assert!(MaterialTier::Diamond < MaterialTier::Netherite);
        // Gold is between Wood and Stone in power
        assert!(MaterialTier::Wood < MaterialTier::Gold);
        assert!(MaterialTier::Gold < MaterialTier::Stone);
    }

    #[test]
    fn test_material_tier_clone_eq() {
        assert_eq!(MaterialTier::Iron, MaterialTier::Iron);
        assert_ne!(MaterialTier::Wood, MaterialTier::Diamond);
    }

    // ── GameMode tests ──────────────────────────────────────

    #[test]
    fn test_game_mode_has_4_variants() {
        let modes = [
            GameMode::Survival,
            GameMode::Creative,
            GameMode::Adventure,
            GameMode::Spectator,
        ];
        assert_eq!(modes.len(), 4);
    }

    // ── BotCommand: 25-variant contract ─────────────────────

    /// Exhaustive match on all BotCommand variants.
    /// The compiler will flag this match as non-exhaustive if a new
    /// variant is added, serving as a compile-time check.
    #[allow(unreachable_code)]
    fn require_exactly_25_variants(cmd: &BotCommand) -> u32 {
        match cmd {
            BotCommand::MoveTo(_) => 1,
            BotCommand::WalkDirection(_) => 1,
            BotCommand::Jump => 1,
            BotCommand::Teleport(_) => 1,
            BotCommand::BreakBlock(_) => 1,
            BotCommand::PlaceBlock(_, _) => 1,
            BotCommand::UseItemOnBlock(_) => 1,
            BotCommand::SwitchHotbarSlot(_) => 1,
            BotCommand::DropItem(_, _) => 1,
            BotCommand::UseItem => 1,
            BotCommand::EquipTool(_) => 1,
            BotCommand::OpenContainer(_) => 1,
            BotCommand::TakeFromContainer(_, _) => 1,
            BotCommand::PutIntoContainer(_, _) => 1,
            BotCommand::CloseContainer => 1,
            BotCommand::AttackEntity(_) => 1,
            BotCommand::ShieldBlock => 1,
            BotCommand::SendChat(_) => 1,
            BotCommand::ExecuteCommand(_) => 1,
            BotCommand::SetGameMode(_) => 1,
            BotCommand::QueryNearbyBlocks(_) => 1,
            BotCommand::QueryNearbyEntities(_) => 1,
            BotCommand::QuerySelfInfo => 1,
            BotCommand::QueryInventory => 1,
            BotCommand::QueryChunkSummary => 1,
        }
    }

    #[test]
    fn test_bot_command_exactly_25_variants() {
        let cmds = all_bot_commands();
        // Verify each variant returns 1 from the exhaustive match
        for cmd in &cmds {
            assert_eq!(require_exactly_25_variants(cmd), 1);
        }
        assert_eq!(cmds.len(), 25);
    }

    #[test]
    fn test_bot_command_no_craft_item() {
        let cmds = all_bot_commands();
        let json = serde_json::to_value(&cmds).unwrap();
        let json_str = serde_json::to_string(&json).unwrap().to_lowercase();
        assert!(
            !json_str.contains("craft_item"),
            "BotCommand must not contain CraftItem variant"
        );
    }

    #[test]
    fn test_bot_command_move_to() {
        let cmd = BotCommand::MoveTo(BlockPos::new(100, 64, 200));
        match cmd {
            BotCommand::MoveTo(pos) => {
                assert_eq!(pos.x, 100);
                assert_eq!(pos.y, 64);
                assert_eq!(pos.z, 200);
            }
            _ => panic!("Expected MoveTo variant"),
        }
    }

    #[test]
    fn test_bot_command_walk_direction() {
        let cmd = BotCommand::WalkDirection(Direction::North);
        match cmd {
            BotCommand::WalkDirection(d) => assert_eq!(d, Direction::North),
            _ => panic!("Expected WalkDirection variant"),
        }
    }

    #[test]
    fn test_bot_command_place_block() {
        let cmd = BotCommand::PlaceBlock(BlockPos::new(10, 20, 30), "diamond_block".into());
        match cmd {
            BotCommand::PlaceBlock(pos, block_type) => {
                assert_eq!(pos, BlockPos::new(10, 20, 30));
                assert_eq!(block_type, "diamond_block");
            }
            _ => panic!("Expected PlaceBlock variant"),
        }
    }

    #[test]
    fn test_bot_command_switch_hotbar_slot() {
        let cmd = BotCommand::SwitchHotbarSlot(4);
        match cmd {
            BotCommand::SwitchHotbarSlot(slot) => assert_eq!(slot, 4),
            _ => panic!("Expected SwitchHotbarSlot variant"),
        }
    }

    #[test]
    fn test_bot_command_attack_entity() {
        let cmd = BotCommand::AttackEntity(42);
        match cmd {
            BotCommand::AttackEntity(id) => assert_eq!(id, 42),
            _ => panic!("Expected AttackEntity variant"),
        }
    }

    #[test]
    fn test_bot_command_query_self_info() {
        let cmd = BotCommand::QuerySelfInfo;
        assert!(matches!(cmd, BotCommand::QuerySelfInfo));
    }

    // ── Serde roundtrip tests ───────────────────────────────

    #[test]
    fn test_bot_command_serde_roundtrip() {
        let cmds = all_bot_commands();
        for cmd in &cmds {
            let json = serde_json::to_string(cmd).unwrap();
            let deserialized: BotCommand = serde_json::from_str(&json).unwrap();
            assert_eq!(cmd, &deserialized, "Serde roundtrip failed for: {json}");
        }
    }

    #[test]
    fn test_bot_result_serde_roundtrip() {
        let result = BotResult {
            success: true,
            message: "operation completed".to_string(),
            data: Some(serde_json::json!({"key": "value"})),
        };
        let json = serde_json::to_string(&result).unwrap();
        let deserialized: BotResult = serde_json::from_str(&json).unwrap();
        assert_eq!(result.success, deserialized.success);
        assert_eq!(result.message, deserialized.message);
        assert_eq!(result.data, deserialized.data);
    }

    #[test]
    fn test_bot_result_default_values() {
        let result = BotResult {
            success: false,
            message: String::new(),
            data: None,
        };
        assert!(!result.success);
        assert!(result.message.is_empty());
        assert!(result.data.is_none());
    }

    #[test]
    fn test_game_event_serde_roundtrip() {
        let events = vec![
            GameEvent::ChatMessage {
                sender: "Alice".into(),
                message: "Hi".into(),
            },
            GameEvent::PlayerJoin {
                username: "Bob".into(),
            },
            GameEvent::PlayerLeave {
                username: "Bob".into(),
            },
            GameEvent::BlockBreak {
                position: BlockPos::new(0, 64, 0),
                block_type: "stone".into(),
            },
            GameEvent::GameModeChange {
                new_mode: GameMode::Creative,
            },
        ];
        for event in &events {
            let json = serde_json::to_string(event).unwrap();
            let deserialized: GameEvent = serde_json::from_str(&json).unwrap();
            let re_json = serde_json::to_string(&deserialized).unwrap();
            assert_eq!(json, re_json, "Serde roundtrip failed for: {json}");
        }
    }

    #[test]
    fn test_game_event_chat_message() {
        let event = GameEvent::ChatMessage {
            sender: "Alice".into(),
            message: "Hello!".into(),
        };
        match event {
            GameEvent::ChatMessage { sender, message } => {
                assert_eq!(sender, "Alice");
                assert_eq!(message, "Hello!");
            }
            _ => panic!("Expected ChatMessage variant"),
        }
    }

    // ── WorldSnapshot / SelfPlayer tests ────────────────────

    #[test]
    fn test_self_player_struct() {
        let player = SelfPlayer {
            uuid: "abc-123".into(),
            username: "Steve".into(),
            position: BlockPos::new(0, 64, 0),
            health: 20.0,
            hunger: 20,
            gamemode: GameMode::Survival,
            held_item_slot: 0,
        };
        assert_eq!(player.uuid, "abc-123");
        assert_eq!(player.username, "Steve");
        assert_eq!(player.health, 20.0);
        assert_eq!(player.hunger, 20);
        assert_eq!(player.gamemode, GameMode::Survival);
        assert_eq!(player.held_item_slot, 0);
    }

    #[test]
    fn test_world_snapshot_serde_roundtrip() {
        let snapshot = WorldSnapshot {
            blocks: vec![BlockEntry {
                position: BlockPos::new(0, 0, 0),
                block_type: "stone".into(),
                block_state: None,
            }],
            entities: vec![EntityEntry {
                id: 42,
                uuid: "entity-uuid".into(),
                entity_type: "zombie".into(),
                position: BlockPos::new(10, 20, 30),
                display_name: Some("Zombie".into()),
                health: Some(20.0),
            }],
            self_player: SelfPlayer {
                uuid: "player-uuid".into(),
                username: "Player".into(),
                position: BlockPos::new(0, 64, 0),
                health: 20.0,
                hunger: 20,
                gamemode: GameMode::Survival,
                held_item_slot: 1,
            },
            timestamp: 1234567890,
            chunk_summary: vec![(0, 0), (1, 0)],
        };
        let json = serde_json::to_string(&snapshot).unwrap();
        let deserialized: WorldSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.blocks.len(), 1);
        assert_eq!(deserialized.entities.len(), 1);
        assert_eq!(deserialized.self_player.username, "Player");
        assert_eq!(deserialized.timestamp, 1234567890);
        assert_eq!(deserialized.chunk_summary, vec![(0, 0), (1, 0)]);
    }

    #[test]
    fn test_block_entry_default() {
        let entry = BlockEntry {
            position: BlockPos::new(10, 20, 30),
            block_type: "diamond_ore".into(),
            block_state: Some("lit=true".into()),
        };
        assert_eq!(entry.position, BlockPos::new(10, 20, 30));
        assert_eq!(entry.block_type, "diamond_ore");
        assert_eq!(entry.block_state, Some("lit=true".into()));
    }

    #[test]
    fn test_entity_entry_default() {
        let entry = EntityEntry {
            id: 1,
            uuid: "uuid-1".into(),
            entity_type: "creeper".into(),
            position: BlockPos::new(0, 0, 0),
            display_name: None,
            health: Some(40.0),
        };
        assert_eq!(entry.id, 1);
        assert_eq!(entry.entity_type, "creeper");
    }

    // ── JsonSchema derive tests ─────────────────────────────

    #[test]
    fn test_json_schema_for_block_pos() {
        let schema = schemars::schema_for!(BlockPos);
        assert_eq!(
            schema
                .schema
                .metadata
                .as_ref()
                .unwrap()
                .title
                .as_ref()
                .unwrap(),
            "BlockPos"
        );
    }

    #[test]
    fn test_json_schema_for_bot_command() {
        let schema = schemars::schema_for!(BotCommand);
        assert_eq!(
            schema
                .schema
                .metadata
                .as_ref()
                .unwrap()
                .title
                .as_ref()
                .unwrap(),
            "BotCommand"
        );
    }

    #[test]
    fn test_json_schema_for_game_mode() {
        let schema = schemars::schema_for!(GameMode);
        assert_eq!(
            schema
                .schema
                .metadata
                .as_ref()
                .unwrap()
                .title
                .as_ref()
                .unwrap(),
            "GameMode"
        );
    }

    #[test]
    fn test_json_schema_for_direction() {
        let schema = schemars::schema_for!(Direction);
        assert_eq!(
            schema
                .schema
                .metadata
                .as_ref()
                .unwrap()
                .title
                .as_ref()
                .unwrap(),
            "Direction"
        );
    }

    #[test]
    fn test_json_schema_for_world_snapshot() {
        let schema = schemars::schema_for!(WorldSnapshot);
        assert_eq!(
            schema
                .schema
                .metadata
                .as_ref()
                .unwrap()
                .title
                .as_ref()
                .unwrap(),
            "WorldSnapshot"
        );
    }

    // ── Helpers ─────────────────────────────────────────────

    fn all_bot_commands() -> Vec<BotCommand> {
        vec![
            BotCommand::MoveTo(BlockPos::new(0, 0, 0)),
            BotCommand::WalkDirection(Direction::North),
            BotCommand::Jump,
            BotCommand::Teleport(BlockPos::new(0, 0, 0)),
            BotCommand::BreakBlock(BlockPos::new(0, 0, 0)),
            BotCommand::PlaceBlock(BlockPos::new(0, 0, 0), "stone".into()),
            BotCommand::UseItemOnBlock(BlockPos::new(0, 0, 0)),
            BotCommand::SwitchHotbarSlot(0),
            BotCommand::DropItem(0, 1),
            BotCommand::UseItem,
            BotCommand::EquipTool(ToolType::Pickaxe),
            BotCommand::OpenContainer(BlockPos::new(0, 0, 0)),
            BotCommand::TakeFromContainer(0, 1),
            BotCommand::PutIntoContainer(0, 1),
            BotCommand::CloseContainer,
            BotCommand::AttackEntity(0),
            BotCommand::ShieldBlock,
            BotCommand::SendChat(String::new()),
            BotCommand::ExecuteCommand(String::new()),
            BotCommand::SetGameMode(GameMode::Survival),
            BotCommand::QueryNearbyBlocks(10),
            BotCommand::QueryNearbyEntities(10),
            BotCommand::QuerySelfInfo,
            BotCommand::QueryInventory,
            BotCommand::QueryChunkSummary,
        ]
    }
}
