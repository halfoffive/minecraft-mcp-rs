//! Error types for the Minecraft MCP server.
//!
//! Defines `BotError`, the unified error enum for all Minecraft bot operations.
//! Each variant carries enough context for AI agents to make informed decisions.

use rmcp::model::{ErrorCode, ErrorData};
use std::fmt::{self, Display, Formatter};

// Re-export the shared position/tool/material types so `BotError` variants and
// the public API share a single definition with `crate::types`. Previously this
// module duplicated these types with incompatible variants (e.g. `error::ToolType`
// had `Hoe` but lacked `Shears`/`Hand`, forcing lossy conversions). Unifying them
// eliminates the `to_error_*` bridge helpers.
pub use crate::types::{BlockPos, MaterialTier, ToolType};

// ---------------------------------------------------------------------------
// BotError
// ---------------------------------------------------------------------------

/// All errors that can occur during Minecraft bot operations.
///
/// Every variant is designed to be *actionable* — an AI consuming this error
/// should be able to decide what to do next based solely on the variant and
/// its attached data.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum BotError {
    /// The bot is not connected to a server.
    Offline(String),

    /// A connection attempt failed.
    ConnectionFailed(String),

    /// A bot command timed out.
    CommandTimeout {
        /// The command that timed out.
        command: String,
        /// Timeout duration in seconds.
        timeout_secs: u64,
    },

    /// A block was not found at the given position.
    BlockNotFound(BlockPos),

    /// The chunk containing the position is not loaded.
    ChunkNotLoaded(BlockPos),

    /// A required tool was not found in the inventory.
    ToolNotFound {
        /// The type of tool needed.
        tool_type: ToolType,
        /// An optional material requirement.
        material: Option<MaterialTier>,
    },

    /// A target position is too far away from the bot.
    TooFar {
        /// The target position.
        target: BlockPos,
        /// The bot's current position.
        current: BlockPos,
        /// Maximum allowed Euclidean distance.
        max_distance: f64,
    },

    /// The inventory is full and cannot accept more items.
    InventoryFull,

    /// Pathfinding to a target position failed.
    PathfindingFailed {
        /// The target position that could not be reached.
        target: BlockPos,
        /// Why pathfinding failed.
        reason: String,
    },

    /// A mining operation was interrupted before completion.
    MiningInterrupted {
        /// Why the mining was interrupted.
        reason: String,
    },

    /// Attempted to open a container when one was already open.
    ContainerAlreadyOpen,

    /// Waiting for a container to open timed out.
    ContainerTimeout,

    /// The operation was denied due to insufficient permissions.
    PermissionDenied(String),

    /// A caller-supplied parameter was invalid (out of range, empty, wrong
    /// type). Maps to MCP `INVALID_PARAMS` so clients can distinguish user
    /// input errors from internal failures.
    InvalidParams(String),

    /// An internal / unexpected error occurred.
    Internal(String),
}

// ---------------------------------------------------------------------------
// Display — manually implemented so ToolNotFound can format Option nicely
// ---------------------------------------------------------------------------

impl Display for BotError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            BotError::Offline(msg) => write!(f, "Bot is offline: {msg}"),
            BotError::ConnectionFailed(msg) => write!(f, "Connection failed: {msg}"),
            BotError::CommandTimeout {
                command,
                timeout_secs,
            } => {
                write!(f, "Command `{command}` timed out after {timeout_secs}s")
            }
            BotError::BlockNotFound(pos) => write!(f, "Block not found at {pos}"),
            BotError::ChunkNotLoaded(pos) => write!(f, "Chunk not loaded at {pos}"),
            BotError::ToolNotFound {
                tool_type,
                material,
            } => match material {
                Some(mat) => write!(f, "Tool not found: {tool_type} ({mat})"),
                None => write!(f, "Tool not found: {tool_type}"),
            },
            BotError::TooFar {
                target,
                current,
                max_distance,
            } => {
                write!(
                    f,
                    "Target {target} is too far from current position {current} (max distance: {max_distance})"
                )
            }
            BotError::InventoryFull => write!(f, "Inventory is full"),
            BotError::PathfindingFailed { target, reason } => {
                write!(f, "Pathfinding failed to {target}: {reason}")
            }
            BotError::MiningInterrupted { reason } => {
                write!(f, "Mining interrupted: {reason}")
            }
            BotError::ContainerAlreadyOpen => write!(f, "A container is already open"),
            BotError::ContainerTimeout => write!(f, "Container open timed out"),
            BotError::PermissionDenied(msg) => write!(f, "Permission denied: {msg}"),
            BotError::InvalidParams(msg) => write!(f, "Invalid parameter: {msg}"),
            BotError::Internal(msg) => write!(f, "Internal error: {msg}"),
        }
    }
}

// thiserror::Error still provides the Error trait impl via the derive macro

// ---------------------------------------------------------------------------
// Conversion to MCP error responses
// ---------------------------------------------------------------------------

impl From<BotError> for ErrorData {
    fn from(err: BotError) -> Self {
        let (code, data) = match &err {
            BotError::Offline(_)
            | BotError::ConnectionFailed(_)
            | BotError::CommandTimeout { .. }
            | BotError::ChunkNotLoaded(_)
            | BotError::InventoryFull
            | BotError::MiningInterrupted { .. }
            | BotError::ContainerAlreadyOpen
            | BotError::ContainerTimeout
            | BotError::Internal(_) => (ErrorCode::INTERNAL_ERROR, None),

            BotError::BlockNotFound(pos) => {
                let detail = serde_json::json!({
                    "x": pos.x,
                    "y": pos.y,
                    "z": pos.z,
                });
                (ErrorCode::RESOURCE_NOT_FOUND, Some(detail))
            }

            BotError::ToolNotFound {
                tool_type,
                material,
            } => {
                let detail = serde_json::json!({
                    "tool_type": tool_type.to_string(),
                    "material": material.as_ref().map(|m| m.to_string()),
                });
                (ErrorCode::INVALID_PARAMS, Some(detail))
            }

            BotError::TooFar {
                target,
                current,
                max_distance,
            } => {
                let detail = serde_json::json!({
                    "target": { "x": target.x, "y": target.y, "z": target.z },
                    "current": { "x": current.x, "y": current.y, "z": current.z },
                    "max_distance": max_distance,
                });
                (ErrorCode::INVALID_PARAMS, Some(detail))
            }

            BotError::PathfindingFailed { target, .. } => {
                let detail = serde_json::json!({
                    "x": target.x,
                    "y": target.y,
                    "z": target.z,
                });
                (ErrorCode::INTERNAL_ERROR, Some(detail))
            }

            BotError::PermissionDenied(_) => (ErrorCode::INVALID_REQUEST, None),

            BotError::InvalidParams(_) => (ErrorCode::INVALID_PARAMS, None),
        };

        ErrorData::new(code, err.to_string(), data)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- Construction / Debug -------------------------------------------------

    #[test]
    fn test_debug_format() {
        let err = BotError::Offline("not connected".into());
        let debug = format!("{err:?}");
        assert!(debug.contains("Offline"));
    }

    #[test]
    fn test_display_offline() {
        let err = BotError::Offline("server unreachable".into());
        assert_eq!(err.to_string(), "Bot is offline: server unreachable");
    }

    #[test]
    fn test_display_connection_failed() {
        let err = BotError::ConnectionFailed("connection refused".into());
        assert_eq!(err.to_string(), "Connection failed: connection refused");
    }

    #[test]
    fn test_display_command_timeout() {
        let err = BotError::CommandTimeout {
            command: "tp".into(),
            timeout_secs: 30,
        };
        assert_eq!(err.to_string(), "Command `tp` timed out after 30s");
    }

    #[test]
    fn test_display_block_not_found() {
        let pos = BlockPos {
            x: 10,
            y: 64,
            z: -20,
        };
        let err = BotError::BlockNotFound(pos);
        assert_eq!(err.to_string(), format!("Block not found at {pos}"));
    }

    #[test]
    fn test_display_chunk_not_loaded() {
        let pos = BlockPos {
            x: 1000,
            y: 0,
            z: 1000,
        };
        let err = BotError::ChunkNotLoaded(pos);
        assert_eq!(err.to_string(), format!("Chunk not loaded at {pos}"));
    }

    #[test]
    fn test_display_tool_not_found() {
        let err = BotError::ToolNotFound {
            tool_type: ToolType::Pickaxe,
            material: Some(MaterialTier::Diamond),
        };
        assert_eq!(err.to_string(), "Tool not found: pickaxe (diamond)");
    }

    #[test]
    fn test_display_tool_not_found_no_material() {
        let err = BotError::ToolNotFound {
            tool_type: ToolType::Sword,
            material: None,
        };
        assert_eq!(err.to_string(), "Tool not found: sword");
    }

    #[test]
    fn test_display_too_far() {
        let target = BlockPos {
            x: 100,
            y: 64,
            z: 0,
        };
        let current = BlockPos { x: 0, y: 64, z: 0 };
        let err = BotError::TooFar {
            target,
            current,
            max_distance: 50.0,
        };
        let msg = err.to_string();
        assert!(msg.contains("100"));
        assert!(msg.contains("max distance"));
    }

    #[test]
    fn test_display_inventory_full() {
        let err = BotError::InventoryFull;
        assert_eq!(err.to_string(), "Inventory is full");
    }

    #[test]
    fn test_display_pathfinding_failed() {
        let target = BlockPos { x: 5, y: 10, z: 15 };
        let err = BotError::PathfindingFailed {
            target,
            reason: "no path".into(),
        };
        assert!(err.to_string().contains("no path"));
    }

    #[test]
    fn test_display_mining_interrupted() {
        let err = BotError::MiningInterrupted {
            reason: "mob attack".into(),
        };
        assert_eq!(err.to_string(), "Mining interrupted: mob attack");
    }

    #[test]
    fn test_display_container_already_open() {
        let err = BotError::ContainerAlreadyOpen;
        assert_eq!(err.to_string(), "A container is already open");
    }

    #[test]
    fn test_display_container_timeout() {
        let err = BotError::ContainerTimeout;
        assert_eq!(err.to_string(), "Container open timed out");
    }

    #[test]
    fn test_display_permission_denied() {
        let err = BotError::PermissionDenied("not operator".into());
        assert_eq!(err.to_string(), "Permission denied: not operator");
    }

    #[test]
    fn test_display_internal() {
        let err = BotError::Internal("something broke".into());
        assert_eq!(err.to_string(), "Internal error: something broke");
    }

    #[test]
    fn test_display_invalid_params() {
        let err = BotError::InvalidParams("hotbar slot 9 out of range".into());
        assert_eq!(
            err.to_string(),
            "Invalid parameter: hotbar slot 9 out of range"
        );
    }

    // -- Clone ----------------------------------------------------------------

    #[test]
    fn test_clone() {
        let err = BotError::InventoryFull;
        assert_eq!(err.clone(), err);

        let err = BotError::Offline("test".into());
        assert_eq!(err.clone(), err);
    }

    // -- Error trait ----------------------------------------------------------

    #[test]
    fn test_error_trait() {
        fn check_source<E: std::error::Error>(_: &E) {}
        let err = BotError::Internal("source test".into());
        check_source(&err);
    }

    // -- Conversion to rmcp::model::ErrorData ---------------------------------

    #[test]
    fn test_into_mcp_error_offline() {
        let err = BotError::Offline("bot is offline".into());
        let mcp: ErrorData = err.into();
        assert_eq!(mcp.code, ErrorCode::INTERNAL_ERROR);
        assert_eq!(mcp.message.as_ref(), "Bot is offline: bot is offline");
    }

    #[test]
    fn test_into_mcp_error_block_not_found() {
        let pos = BlockPos { x: 1, y: 2, z: 3 };
        let err = BotError::BlockNotFound(pos);
        let mcp: ErrorData = err.into();
        assert_eq!(mcp.code, ErrorCode::RESOURCE_NOT_FOUND);
        assert!(mcp.data.is_some());
        let data = mcp.data.unwrap();
        assert_eq!(data["x"], 1);
        assert_eq!(data["y"], 2);
        assert_eq!(data["z"], 3);
    }

    #[test]
    fn test_into_mcp_error_tool_not_found() {
        let err = BotError::ToolNotFound {
            tool_type: ToolType::Axe,
            material: Some(MaterialTier::Iron),
        };
        let mcp: ErrorData = err.into();
        assert_eq!(mcp.code, ErrorCode::INVALID_PARAMS);
        let data = mcp.data.unwrap();
        assert_eq!(data["tool_type"], "axe");
        assert_eq!(data["material"], "iron");
    }

    #[test]
    fn test_into_mcp_error_too_far() {
        let err = BotError::TooFar {
            target: BlockPos {
                x: 10,
                y: 20,
                z: 30,
            },
            current: BlockPos { x: 0, y: 0, z: 0 },
            max_distance: 42.0,
        };
        let mcp: ErrorData = err.into();
        assert_eq!(mcp.code, ErrorCode::INVALID_PARAMS);
        let data = mcp.data.unwrap();
        assert_eq!(data["max_distance"], 42.0);
    }

    #[test]
    fn test_into_mcp_error_permission_denied() {
        let err = BotError::PermissionDenied("no access".into());
        let mcp: ErrorData = err.into();
        assert_eq!(mcp.code, ErrorCode::INVALID_REQUEST);
    }

    #[test]
    fn test_into_mcp_error_internal() {
        let err = BotError::Internal("unexpected".into());
        let mcp: ErrorData = err.into();
        assert_eq!(mcp.code, ErrorCode::INTERNAL_ERROR);
    }

    #[test]
    fn test_into_mcp_error_invalid_params() {
        let err = BotError::InvalidParams("slot out of range".into());
        let mcp: ErrorData = err.into();
        assert_eq!(mcp.code, ErrorCode::INVALID_PARAMS);
    }

    #[test]
    fn test_into_mcp_error_inventory_full() {
        let err = BotError::InventoryFull;
        let mcp: ErrorData = err.into();
        assert_eq!(mcp.code, ErrorCode::INTERNAL_ERROR);
        assert_eq!(mcp.message.as_ref(), "Inventory is full");
    }
}
