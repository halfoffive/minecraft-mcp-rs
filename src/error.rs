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

    /// A bot command timed out.
    CommandTimeout {
        /// The command that timed out.
        command: String,
        /// Timeout duration in seconds.
        timeout_secs: u64,
    },

    /// A block was not found at the given position.
    BlockNotFound(BlockPos),

    /// The requested entity was not found in the current world snapshot.
    ///
    /// A not-found condition, not a malformed parameter: the ID is a valid
    /// Minecraft entity ID, but the snapshot no longer contains it (stale or
    /// fabricated ID). Maps to `RESOURCE_NOT_FOUND` so MCP clients can branch
    /// on the error code instead of parsing `InvalidParams` messages (F-34).
    EntityNotFound(u32),

    /// The chunk containing the position is not loaded.
    ChunkNotLoaded(BlockPos),

    /// A required tool was not found in the inventory.
    ToolNotFound {
        /// The type of tool needed.
        tool_type: ToolType,
        /// An optional material requirement.
        material: Option<MaterialTier>,
        /// Suggested alternatives (e.g. ["Iron Pickaxe"]).
        alternatives: Vec<String>,
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

    /// No container is currently open (a runtime-state error: the caller
    /// must `open_container` first).
    ///
    /// Distinguished from [`InvalidParams`](Self::InvalidParams) — the
    /// parameters were fine; the *state* is wrong. Maps to its own JSON-RPC
    /// code (-32010) so MCP clients can branch on it (audit L-9).
    ContainerNotOpen,

    /// The operation was denied due to insufficient permissions.
    PermissionDenied(String),

    /// A caller-supplied parameter was invalid (out of range, empty, wrong
    /// type). Maps to MCP `INVALID_PARAMS` so clients can distinguish user
    /// input errors from internal failures.
    InvalidParams(String),

    /// The server rejected a command sent via chat (e.g. `execute_command`).
    ///
    /// The server reports failures like `Incorrect argument for command ...`
    /// as a chat message rather than an error packet; the executor detects
    /// that feedback after sending and surfaces it here so the MCP client
    /// learns the command was actually rejected instead of seeing a fake
    /// success. `feedback` is the server's rejection message verbatim.
    CommandRejected {
        /// The command that was rejected (as sent, with leading `/`).
        command: String,
        /// The server's rejection feedback (chat/system message).
        feedback: String,
    },

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
            BotError::CommandTimeout {
                command,
                timeout_secs,
            } => {
                write!(f, "Command `{command}` timed out after {timeout_secs}s")
            }
            BotError::BlockNotFound(pos) => write!(f, "Block not found at {pos}"),
            BotError::EntityNotFound(id) => write!(f, "Entity not found with id {id}"),
            BotError::ChunkNotLoaded(pos) => write!(f, "Chunk not loaded at {pos}"),
            BotError::ToolNotFound {
                tool_type,
                material,
                alternatives,
            } => {
                let mut msg = match material {
                    Some(mat) => format!("Tool not found: {tool_type} ({mat})"),
                    None => format!("Tool not found: {tool_type}"),
                };
                if !alternatives.is_empty() {
                    msg.push_str("; use ");
                    msg.push_str(&alternatives.join(" or "));
                }
                write!(f, "{msg}")
            }
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
            BotError::ContainerNotOpen => write!(f, "No container is currently open"),
            BotError::PermissionDenied(msg) => write!(f, "Permission denied: {msg}"),
            BotError::InvalidParams(msg) => write!(f, "Invalid parameter: {msg}"),
            BotError::CommandRejected { command, feedback } => {
                write!(
                    f,
                    "Command `{command}` was rejected by the server: {feedback}"
                )
            }
            BotError::Internal(msg) => write!(f, "Internal error: {msg}"),
        }
    }
}

// thiserror::Error still provides the Error trait impl via the derive macro

// ---------------------------------------------------------------------------
// Conversion to MCP error responses
// ---------------------------------------------------------------------------

// Custom JSON-RPC server error codes.
//
// JSON-RPC reserves `-32000..=-32099` for implementation-defined server
// errors; every bot-specific failure gets a DISTINCT code so MCP clients can
// branch on `error.code` alone. `-32002` is deliberately skipped — it is
// rmcp's `ErrorCode::RESOURCE_NOT_FOUND` (used by `BlockNotFound`).
//
// Error-contract table (every variant carries structured `data` with a
// machine-readable snake_case `reason`, a `retryable` bool, and the listed
// variant-specific fields):
//
// | Code   | Constant / std code              | BotError variant       | reason               | retryable | extra data fields                    |
// |--------|----------------------------------|------------------------|----------------------|-----------|--------------------------------------|
// | -32000 | `CODE_OFFLINE`                   | `Offline`              | `bot_disconnected`   | true      | —                                    |
// | -32001 | `CODE_COMMAND_TIMEOUT`           | `CommandTimeout`       | `command_timeout`    | true      | `command`, `timeout_secs`            |
// | -32002 | `ErrorCode::RESOURCE_NOT_FOUND`  | `BlockNotFound`        | `block_not_found`    | false     | `x`, `y`, `z`                        |
// | -32002 | `ErrorCode::RESOURCE_NOT_FOUND`  | `EntityNotFound`       | `entity_not_found`   | false     | `entity_id`                          |
// | -32003 | `CODE_CHUNK_NOT_LOADED`          | `ChunkNotLoaded`       | `chunk_not_loaded`   | true      | `x`, `y`, `z`                        |
// | -32004 | `CODE_INVENTORY_FULL`            | `InventoryFull`        | `inventory_full`     | false     | —                                    |
// | -32005 | `CODE_MINING_INTERRUPTED`        | `MiningInterrupted`    | `mining_interrupted` | false     | `detail`                             |
// | -32006 | `CODE_CONTAINER_ALREADY_OPEN`    | `ContainerAlreadyOpen` | `container_already_open` | false | —                                |
// | -32007 | `CODE_CONTAINER_TIMEOUT`         | `ContainerTimeout`     | `container_timeout`  | true      | —                                    |
// | -32008 | `CODE_PATHFINDING_FAILED`        | `PathfindingFailed`    | `pathfinding_failed` | false     | `x`, `y`, `z` (target), `detail`     |
// | -32009 | `CODE_COMMAND_REJECTED`          | `CommandRejected`      | `command_rejected`   | true      | `command`, `feedback`                |
// | -32010 | `CODE_CONTAINER_NOT_OPEN`        | `ContainerNotOpen`     | `container_not_open` | false     | —                                    |
// | -32600 | `ErrorCode::INVALID_REQUEST`     | `PermissionDenied`     | `permission_denied`  | false     | —                                    |
// | -32602 | `ErrorCode::INVALID_PARAMS`      | `ToolNotFound`         | `tool_not_found`     | false     | `tool_type`, `material`, `alternatives` |
// | -32602 | `ErrorCode::INVALID_PARAMS`      | `TooFar`               | `too_far`            | false     | `target`, `current`, `max_distance`  |
// | -32602 | `ErrorCode::INVALID_PARAMS`      | `InvalidParams`        | `invalid_params`     | false     | —                                    |
// | -32603 | `ErrorCode::INTERNAL_ERROR`      | `Internal`             | `internal_error`     | false     | —                                    |
//
// Variants sharing a standard code (`INVALID_PARAMS`) remain distinguishable
// via `data.reason`.
const CODE_OFFLINE: i32 = -32000;
const CODE_COMMAND_TIMEOUT: i32 = -32001;
// -32002 is rmcp's RESOURCE_NOT_FOUND — intentionally not redefined here.
const CODE_CHUNK_NOT_LOADED: i32 = -32003;
const CODE_INVENTORY_FULL: i32 = -32004;
const CODE_MINING_INTERRUPTED: i32 = -32005;
const CODE_CONTAINER_ALREADY_OPEN: i32 = -32006;
const CODE_CONTAINER_TIMEOUT: i32 = -32007;
const CODE_PATHFINDING_FAILED: i32 = -32008;
const CODE_COMMAND_REJECTED: i32 = -32009;
const CODE_CONTAINER_NOT_OPEN: i32 = -32010;

impl From<BotError> for ErrorData {
    fn from(err: BotError) -> Self {
        // Every arm produces a structured `data` payload; see the contract
        // table above the `CODE_*` constants.
        let (code, data) = match &err {
            // `Offline` is mapped to a custom JSON-RPC server error
            // (-32000) rather than `INTERNAL_ERROR` (-32603) so MCP
            // clients can distinguish "the bot is not connected, retry
            // after connecting" from genuine server-side bugs. The
            // `data.reason` field carries a machine-readable string
            // for clients that want to surface it in the UI.
            BotError::Offline(_) => (
                // JSON-RPC reserves -32000..=-32099 for
                // implementation-defined server errors. `ErrorCode` is a
                // public tuple struct (no `new` constructor), so we
                // build the value directly.
                ErrorCode(CODE_OFFLINE),
                serde_json::json!({
                    "reason": "bot_disconnected",
                    "retryable": true,
                }),
            ),

            BotError::CommandTimeout {
                command,
                timeout_secs,
            } => (
                ErrorCode(CODE_COMMAND_TIMEOUT),
                serde_json::json!({
                    "reason": "command_timeout",
                    "retryable": true,
                    "command": command,
                    "timeout_secs": timeout_secs,
                }),
            ),

            BotError::BlockNotFound(pos) => (
                ErrorCode::RESOURCE_NOT_FOUND,
                serde_json::json!({
                    "reason": "block_not_found",
                    "retryable": false,
                    "x": pos.x,
                    "y": pos.y,
                    "z": pos.z,
                }),
            ),

            BotError::EntityNotFound(entity_id) => (
                ErrorCode::RESOURCE_NOT_FOUND,
                serde_json::json!({
                    "reason": "entity_not_found",
                    "retryable": false,
                    "entity_id": entity_id,
                }),
            ),

            BotError::ChunkNotLoaded(pos) => (
                ErrorCode(CODE_CHUNK_NOT_LOADED),
                serde_json::json!({
                    "reason": "chunk_not_loaded",
                    "retryable": true,
                    "x": pos.x,
                    "y": pos.y,
                    "z": pos.z,
                }),
            ),

            BotError::ToolNotFound {
                tool_type,
                material,
                alternatives,
            } => (
                ErrorCode::INVALID_PARAMS,
                serde_json::json!({
                    "reason": "tool_not_found",
                    "retryable": false,
                    "tool_type": tool_type.to_string(),
                    "material": material.as_ref().map(|m| m.to_string()),
                    "alternatives": alternatives,
                }),
            ),

            BotError::TooFar {
                target,
                current,
                max_distance,
            } => (
                ErrorCode::INVALID_PARAMS,
                serde_json::json!({
                    "reason": "too_far",
                    "retryable": false,
                    "target": { "x": target.x, "y": target.y, "z": target.z },
                    "current": { "x": current.x, "y": current.y, "z": current.z },
                    "max_distance": max_distance,
                }),
            ),

            BotError::InventoryFull => (
                ErrorCode(CODE_INVENTORY_FULL),
                serde_json::json!({
                    "reason": "inventory_full",
                    "retryable": false,
                }),
            ),

            BotError::PathfindingFailed { target, reason } => (
                ErrorCode(CODE_PATHFINDING_FAILED),
                serde_json::json!({
                    "reason": "pathfinding_failed",
                    "retryable": false,
                    // Target coordinates stay flat (pre-existing wire
                    // shape); the failure explanation travels as `detail`
                    // because `reason` is the variant discriminator.
                    "x": target.x,
                    "y": target.y,
                    "z": target.z,
                    "detail": reason,
                }),
            ),

            BotError::MiningInterrupted { reason } => (
                ErrorCode(CODE_MINING_INTERRUPTED),
                serde_json::json!({
                    "reason": "mining_interrupted",
                    "retryable": false,
                    // The interrupt reason travels as `detail` because
                    // `reason` is the variant discriminator.
                    "detail": reason,
                }),
            ),

            BotError::ContainerAlreadyOpen => (
                ErrorCode(CODE_CONTAINER_ALREADY_OPEN),
                serde_json::json!({
                    "reason": "container_already_open",
                    "retryable": false,
                }),
            ),

            BotError::ContainerTimeout => (
                ErrorCode(CODE_CONTAINER_TIMEOUT),
                serde_json::json!({
                    "reason": "container_timeout",
                    "retryable": true,
                }),
            ),

            BotError::ContainerNotOpen => (
                // L-9: distinct runtime-state code so clients can branch on
                // `error.code` alone — "open a container first" is not an
                // invalid parameter.
                ErrorCode(CODE_CONTAINER_NOT_OPEN),
                serde_json::json!({
                    "reason": "container_not_open",
                    "retryable": false,
                }),
            ),

            BotError::PermissionDenied(_) => (
                ErrorCode::INVALID_REQUEST,
                serde_json::json!({
                    "reason": "permission_denied",
                    "retryable": false,
                }),
            ),

            BotError::InvalidParams(_) => (
                ErrorCode::INVALID_PARAMS,
                serde_json::json!({
                    "reason": "invalid_params",
                    "retryable": false,
                }),
            ),

            BotError::CommandRejected { command, feedback } => (
                ErrorCode(CODE_COMMAND_REJECTED),
                serde_json::json!({
                    "reason": "command_rejected",
                    "retryable": true,
                    "command": command,
                    "feedback": feedback,
                }),
            ),

            BotError::Internal(_) => (
                ErrorCode::INTERNAL_ERROR,
                serde_json::json!({
                    "reason": "internal_error",
                    "retryable": false,
                }),
            ),
        };

        ErrorData::new(code, err.to_string(), Some(data))
    }
}

// F-32: this is a deliberate project contract, documented in the table above:
// tool-level failures travel as JSON-RPC `ErrorData` with structured `reason`
// codes instead of an MCP `is_error: true` `CallToolResult`. The error code
// is branchable and every payload carries `reason` + `retryable`; MCP client
// authors should read `error.data.reason`, not only `error.code`.
impl rmcp::handler::server::tool::IntoCallToolResult for BotError {
    fn into_call_tool_result(self) -> Result<rmcp::model::CallToolResult, ErrorData> {
        Err(self.into())
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
    fn test_display_entity_not_found() {
        let err = BotError::EntityNotFound(42);
        assert_eq!(err.to_string(), "Entity not found with id 42");
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
            alternatives: vec![],
        };
        assert_eq!(err.to_string(), "Tool not found: pickaxe (diamond)");
    }

    #[test]
    fn test_display_tool_not_found_no_material() {
        let err = BotError::ToolNotFound {
            tool_type: ToolType::Sword,
            material: None,
            alternatives: vec![],
        };
        assert_eq!(err.to_string(), "Tool not found: sword");
    }

    #[test]
    fn test_display_tool_not_found_with_alternatives() {
        let err = BotError::ToolNotFound {
            tool_type: ToolType::Pickaxe,
            material: None,
            alternatives: vec!["Iron Pickaxe".to_string(), "Diamond Pickaxe".to_string()],
        };
        assert_eq!(
            err.to_string(),
            "Tool not found: pickaxe; use Iron Pickaxe or Diamond Pickaxe"
        );
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
    fn test_display_container_not_open() {
        let err = BotError::ContainerNotOpen;
        assert_eq!(err.to_string(), "No container is currently open");
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

    #[test]
    fn test_display_command_rejected() {
        let err = BotError::CommandRejected {
            command: "/item replace entity @s hotbar.0 dirt 64".into(),
            feedback: "Incorrect argument for command".into(),
        };
        let msg = err.to_string();
        assert!(msg.contains("rejected by the server"));
        assert!(msg.contains("/item replace"));
        assert!(msg.contains("Incorrect argument"));
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
    //
    // Contract: EVERY `BotError` variant maps to a distinct error code and
    // carries a structured `data` payload with the mandatory machine-readable
    // fields `reason` (snake_case variant discriminator) and `retryable`
    // (bool), plus variant-specific fields. These tests assert LITERAL codes
    // (not the `CODE_*` constants) so a typo in a constant cannot make its
    // own test pass.

    /// Extract the structured `data` payload and assert the two mandatory
    /// contract fields (`reason` + `retryable`); returns the payload so
    /// callers can additionally assert variant-specific fields.
    fn assert_contract(
        mcp: &ErrorData,
        expected_reason: &str,
        expected_retryable: bool,
    ) -> serde_json::Value {
        let data = mcp
            .data
            .as_ref()
            .expect("every BotError variant must carry a structured data payload")
            .clone();
        assert_eq!(
            data["reason"], expected_reason,
            "wrong `reason` for {expected_reason}"
        );
        assert_eq!(
            data["retryable"], expected_retryable,
            "wrong `retryable` for {expected_reason}"
        );
        data
    }

    #[test]
    fn test_into_mcp_error_offline() {
        let err = BotError::Offline("bot is offline".into());
        let mcp: ErrorData = err.into();
        // BotError::Offline maps to a custom JSON-RPC server error
        // (-32000) — NOT INTERNAL_ERROR (-32603) — so MCP clients can
        // distinguish "the bot is not connected" from genuine
        // server-side bugs. The human-readable message is unchanged
        // (clients surface it as `error.message`).
        assert_eq!(mcp.code.0, -32000);
        assert_eq!(mcp.message.as_ref(), "Bot is offline: bot is offline");
        // The `data` payload carries a machine-readable `reason` for
        // clients that want to render a tailored UI hint (e.g. a
        // "Connect the bot" call-to-action).
        assert_contract(&mcp, "bot_disconnected", true);
    }

    #[test]
    fn test_into_mcp_error_command_timeout() {
        let err = BotError::CommandTimeout {
            command: "mine".into(),
            timeout_secs: 30,
        };
        let mcp: ErrorData = err.into();
        // Regression guard: CommandTimeout used to map to INTERNAL_ERROR
        // with no data payload; it now has its own code and carries the
        // timed-out command plus the configured timeout.
        assert_ne!(mcp.code, ErrorCode::INTERNAL_ERROR);
        assert_eq!(mcp.code.0, -32001);
        let data = assert_contract(&mcp, "command_timeout", true);
        assert_eq!(data["command"], "mine");
        assert_eq!(data["timeout_secs"], 30);
    }

    #[test]
    fn test_into_mcp_error_block_not_found() {
        let pos = BlockPos { x: 1, y: 2, z: 3 };
        let err = BotError::BlockNotFound(pos);
        let mcp: ErrorData = err.into();
        assert_eq!(mcp.code, ErrorCode::RESOURCE_NOT_FOUND);
        let data = assert_contract(&mcp, "block_not_found", false);
        assert_eq!(data["x"], 1);
        assert_eq!(data["y"], 2);
        assert_eq!(data["z"], 3);
    }

    #[test]
    fn test_into_mcp_error_entity_not_found() {
        let err = BotError::EntityNotFound(42);
        let mcp: ErrorData = err.into();
        assert_eq!(mcp.code, ErrorCode::RESOURCE_NOT_FOUND);
        let data = assert_contract(&mcp, "entity_not_found", false);
        assert_eq!(data["entity_id"], 42);
    }

    #[test]
    fn test_into_mcp_error_chunk_not_loaded() {
        let pos = BlockPos {
            x: 16,
            y: 64,
            z: -16,
        };
        let err = BotError::ChunkNotLoaded(pos);
        let mcp: ErrorData = err.into();
        assert_eq!(mcp.code.0, -32003);
        let data = assert_contract(&mcp, "chunk_not_loaded", true);
        assert_eq!(data["x"], 16);
        assert_eq!(data["y"], 64);
        assert_eq!(data["z"], -16);
    }

    #[test]
    fn test_into_mcp_error_tool_not_found() {
        let err = BotError::ToolNotFound {
            tool_type: ToolType::Axe,
            material: Some(MaterialTier::Iron),
            alternatives: vec!["Iron Axe".to_string()],
        };
        let mcp: ErrorData = err.into();
        assert_eq!(mcp.code, ErrorCode::INVALID_PARAMS);
        let data = assert_contract(&mcp, "tool_not_found", false);
        assert_eq!(data["tool_type"], "axe");
        assert_eq!(data["material"], "iron");
        // `alternatives` (upstream PR #21) must stay in the payload.
        assert_eq!(data["alternatives"][0], "Iron Axe");
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
        let data = assert_contract(&mcp, "too_far", false);
        assert_eq!(data["max_distance"], 42.0);
        assert_eq!(data["target"]["x"], 10);
        assert_eq!(data["current"]["z"], 0);
    }

    #[test]
    fn test_into_mcp_error_inventory_full() {
        let err = BotError::InventoryFull;
        let mcp: ErrorData = err.into();
        // Regression guard: used to be INTERNAL_ERROR with no data payload.
        assert_ne!(mcp.code, ErrorCode::INTERNAL_ERROR);
        assert_eq!(mcp.code.0, -32004);
        assert_eq!(mcp.message.as_ref(), "Inventory is full");
        assert_contract(&mcp, "inventory_full", false);
    }

    #[test]
    fn test_into_mcp_error_pathfinding_failed() {
        let err = BotError::PathfindingFailed {
            target: BlockPos { x: 5, y: 10, z: 15 },
            reason: "no path".into(),
        };
        let mcp: ErrorData = err.into();
        // Regression guard: used to be INTERNAL_ERROR.
        assert_ne!(mcp.code, ErrorCode::INTERNAL_ERROR);
        assert_eq!(mcp.code.0, -32008);
        let data = assert_contract(&mcp, "pathfinding_failed", false);
        // Target coordinates stay flat (existing wire shape); the failure
        // explanation travels under `detail`.
        assert_eq!(data["x"], 5);
        assert_eq!(data["y"], 10);
        assert_eq!(data["z"], 15);
        assert_eq!(data["detail"], "no path");
    }

    #[test]
    fn test_into_mcp_error_mining_interrupted() {
        let err = BotError::MiningInterrupted {
            reason: "block still present".into(),
        };
        let mcp: ErrorData = err.into();
        assert_eq!(mcp.code.0, -32005);
        let data = assert_contract(&mcp, "mining_interrupted", false);
        // The interrupt reason travels under `detail` — the `reason` key is
        // reserved for the machine-readable variant discriminator.
        assert_eq!(data["detail"], "block still present");
    }

    #[test]
    fn test_into_mcp_error_container_already_open() {
        let err = BotError::ContainerAlreadyOpen;
        let mcp: ErrorData = err.into();
        assert_eq!(mcp.code.0, -32006);
        assert_contract(&mcp, "container_already_open", false);
    }

    #[test]
    fn test_into_mcp_error_container_timeout() {
        let err = BotError::ContainerTimeout;
        let mcp: ErrorData = err.into();
        assert_eq!(mcp.code.0, -32007);
        assert_contract(&mcp, "container_timeout", true);
    }

    #[test]
    fn test_into_mcp_error_container_not_open() {
        let err = BotError::ContainerNotOpen;
        let mcp: ErrorData = err.into();
        // L-9: "no container is currently open" is a RUNTIME state error, not
        // a parameter error — it gets a distinct code (-32010) so MCP clients
        // can distinguish "open a container first" from "your input is
        // invalid" (which shares -32602 with other parameter errors).
        assert_eq!(mcp.code.0, -32010);
        assert_contract(&mcp, "container_not_open", false);
    }

    #[test]
    fn test_into_mcp_error_permission_denied() {
        let err = BotError::PermissionDenied("no access".into());
        let mcp: ErrorData = err.into();
        assert_eq!(mcp.code, ErrorCode::INVALID_REQUEST);
        assert_contract(&mcp, "permission_denied", false);
    }

    #[test]
    fn test_into_mcp_error_invalid_params() {
        let err = BotError::InvalidParams("slot out of range".into());
        let mcp: ErrorData = err.into();
        assert_eq!(mcp.code, ErrorCode::INVALID_PARAMS);
        assert_contract(&mcp, "invalid_params", false);
    }

    #[test]
    fn test_into_mcp_error_internal() {
        let err = BotError::Internal("unexpected".into());
        let mcp: ErrorData = err.into();
        assert_eq!(mcp.code, ErrorCode::INTERNAL_ERROR);
        assert_contract(&mcp, "internal_error", false);
    }

    #[test]
    fn test_into_mcp_error_command_rejected() {
        let err = BotError::CommandRejected {
            command: "/item replace entity @s hotbar.0 dirt 64".into(),
            feedback: "Incorrect argument for command ... dirt 64<--[HERE]".into(),
        };
        let mcp: ErrorData = err.into();
        // Distinct code so clients can branch on `error.code` alone —
        // "the server rejected the command" is different from a timeout
        // (-32001) or a generic internal error (-32603).
        assert_eq!(mcp.code.0, -32009);
        let data = assert_contract(&mcp, "command_rejected", true);
        assert_eq!(data["command"], "/item replace entity @s hotbar.0 dirt 64");
        assert!(
            data["feedback"]
                .as_str()
                .unwrap()
                .contains("Incorrect argument")
        );
        // The human-readable message must surface the server feedback.
        assert!(mcp.message.as_ref().contains("rejected by the server"));
    }

    #[test]
    fn test_into_mcp_error_command_rejected_message_contains_feedback() {
        let err = BotError::CommandRejected {
            command: "/give @s dirt 1".into(),
            feedback: "Unknown command".into(),
        };
        let mcp: ErrorData = err.into();
        assert_eq!(mcp.code.0, -32009);
        assert!(mcp.message.as_ref().contains("Unknown command"));
    }

    #[test]
    fn test_custom_error_codes_are_distinct_and_in_reserved_range() {
        // Guard the contract table: every custom code is unique, lies in
        // the JSON-RPC implementation-defined range (-32000..=-32099), and
        // never reuses -32002 (rmcp's `RESOURCE_NOT_FOUND`).
        let codes = [
            CODE_OFFLINE,
            CODE_COMMAND_TIMEOUT,
            CODE_CHUNK_NOT_LOADED,
            CODE_INVENTORY_FULL,
            CODE_MINING_INTERRUPTED,
            CODE_CONTAINER_ALREADY_OPEN,
            CODE_CONTAINER_TIMEOUT,
            CODE_PATHFINDING_FAILED,
            CODE_COMMAND_REJECTED,
            CODE_CONTAINER_NOT_OPEN,
        ];
        for (i, code) in codes.iter().enumerate() {
            assert!(
                (-32099..=-32000).contains(code),
                "code {code} outside the reserved implementation-defined range"
            );
            assert_ne!(
                *code,
                ErrorCode::RESOURCE_NOT_FOUND.0,
                "code must not reuse rmcp's RESOURCE_NOT_FOUND"
            );
            for other in codes.iter().skip(i + 1) {
                assert_ne!(code, other, "custom error codes must be distinct");
            }
        }
    }
}
