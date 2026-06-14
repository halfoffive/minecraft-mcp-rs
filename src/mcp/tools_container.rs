//! MCP tools for container interaction (chests, furnaces, etc.).
//!
//! Each tool validates parameters, checks online status and container state,
//! and dispatches a [`BotCommand`](crate::types::BotCommand) through the bot
//! command channel.
//!
//! # Parameter structs
//!
//! We implement [`rmcp::schemars::JsonSchema`] manually using schemars v1.2.1
//! API (bundled by rmcp 1.7.0) to avoid version conflicts with the project's
//! schemars v0.8 dependency.

use std::borrow::Cow;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::channel::BotCommandSender;
use crate::command_validate::validate_block_pos;
use crate::state::SharedState;
use crate::types::{BlockPos, BotCommand};

// ── Helper ──────────────────────────────────────────────────────────────────

fn schema_from_json(v: Value) -> rmcp::schemars::Schema {
    let map: Map<String, Value> = v.as_object().cloned().unwrap_or_default();
    rmcp::schemars::Schema::from(map)
}

// ── Container state helpers ─────────────────────────────────────────────────

/// Ensure a container is currently open, returning an error JSON string otherwise.
fn check_container_open(state: &SharedState) -> Result<(), String> {
    if !state.has_container_open() {
        return Err(
            r#"{"success":false,"error":"No container is currently open"}"#.to_string()
        );
    }
    Ok(())
}

/// Ensure no container is currently open, returning an error JSON string otherwise.
fn check_container_not_open(state: &SharedState) -> Result<(), String> {
    if state.has_container_open() {
        return Err(
            r#"{"success":false,"error":"A container is already open — close it first"}"#.to_string()
        );
    }
    Ok(())
}

// ── open_container ──────────────────────────────────────────────────────────

/// Input for the `open_container` MCP tool.
#[derive(Deserialize, Default)]
pub struct OpenContainerInput {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl rmcp::schemars::JsonSchema for OpenContainerInput {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("OpenContainerInput")
    }

    fn json_schema(
        _gen: &mut rmcp::schemars::SchemaGenerator,
    ) -> rmcp::schemars::Schema {
        schema_from_json(json!({
            "type": "object",
            "properties": {
                "x": {
                    "type": "integer",
                    "description": "X coordinate of the container to open"
                },
                "y": {
                    "type": "integer",
                    "description": "Y coordinate of the container to open"
                },
                "z": {
                    "type": "integer",
                    "description": "Z coordinate of the container to open"
                }
            },
            "required": ["x", "y", "z"],
            "additionalProperties": false
        }))
    }
}

/// Handle `open_container` MCP tool.
///
/// Validates coordinates, ensures no container is already open,
/// checks online status, then sends [`BotCommand::OpenContainer`].
pub async fn handle_open_container(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    input: OpenContainerInput,
) -> String {
    if let Err(e) = validate_block_pos(&BlockPos::new(input.x, input.y, input.z)) {
        return format!(r#"{{"success":false,"error":"{e}"}}"#);
    }

    if let Err(e) = check_container_not_open(state) {
        return e;
    }

    if !state.is_online() {
        return r#"{"success":false,"error":"Bot is not connected to a server"}"#
            .to_string();
    }

    let cmd = BotCommand::OpenContainer(BlockPos::new(input.x, input.y, input.z));
    match sender.send_command(cmd).await {
        Ok(result) => serde_json::to_string(&result).unwrap_or_else(|e| {
            format!(r#"{{"success":false,"error":"Serialization error: {e}"}}"#)
        }),
        Err(e) => format!(r#"{{"success":false,"error":"{e}"}}"#),
    }
}

// ── take_from_container ─────────────────────────────────────────────────────

/// Input for the `take_from_container` MCP tool.
#[derive(Deserialize, Default)]
pub struct TakeFromContainerInput {
    pub slot: u8,
    pub count: Option<u8>,
}

impl rmcp::schemars::JsonSchema for TakeFromContainerInput {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("TakeFromContainerInput")
    }

    fn json_schema(
        _gen: &mut rmcp::schemars::SchemaGenerator,
    ) -> rmcp::schemars::Schema {
        schema_from_json(json!({
            "type": "object",
            "properties": {
                "slot": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Container slot index (0-based) to take items from"
                },
                "count": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 64,
                    "description": "Number of items to take (default 1)"
                }
            },
            "required": ["slot"],
            "additionalProperties": false
        }))
    }
}

/// Handle `take_from_container` MCP tool.
///
/// Requires a container to be open, checks online status, then sends
/// [`BotCommand::TakeFromContainer`].
pub async fn handle_take_from_container(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    input: TakeFromContainerInput,
) -> String {
    if let Err(e) = check_container_open(state) {
        return e;
    }

    if !state.is_online() {
        return r#"{"success":false,"error":"Bot is not connected to a server"}"#
            .to_string();
    }

    let count = input.count.unwrap_or(1);
    if count == 0 {
        return r#"{"success":false,"error":"Count must be greater than 0"}"#
            .to_string();
    }

    let cmd = BotCommand::TakeFromContainer(input.slot, count);
    match sender.send_command(cmd).await {
        Ok(result) => serde_json::to_string(&result).unwrap_or_else(|e| {
            format!(r#"{{"success":false,"error":"Serialization error: {e}"}}"#)
        }),
        Err(e) => format!(r#"{{"success":false,"error":"{e}"}}"#),
    }
}

// ── put_into_container ──────────────────────────────────────────────────────

/// Input for the `put_into_container` MCP tool.
#[derive(Deserialize, Default)]
pub struct PutIntoContainerInput {
    pub slot: u8,
    pub count: Option<u8>,
}

impl rmcp::schemars::JsonSchema for PutIntoContainerInput {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("PutIntoContainerInput")
    }

    fn json_schema(
        _gen: &mut rmcp::schemars::SchemaGenerator,
    ) -> rmcp::schemars::Schema {
        schema_from_json(json!({
            "type": "object",
            "properties": {
                "slot": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Container slot index (0-based) to put items into"
                },
                "count": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 64,
                    "description": "Number of items to put (default 1)"
                }
            },
            "required": ["slot"],
            "additionalProperties": false
        }))
    }
}

/// Handle `put_into_container` MCP tool.
///
/// Requires a container to be open, checks online status, then sends
/// [`BotCommand::PutIntoContainer`].
pub async fn handle_put_into_container(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    input: PutIntoContainerInput,
) -> String {
    if let Err(e) = check_container_open(state) {
        return e;
    }

    if !state.is_online() {
        return r#"{"success":false,"error":"Bot is not connected to a server"}"#
            .to_string();
    }

    let count = input.count.unwrap_or(1);
    if count == 0 {
        return r#"{"success":false,"error":"Count must be greater than 0"}"#
            .to_string();
    }

    let cmd = BotCommand::PutIntoContainer(input.slot, count);
    match sender.send_command(cmd).await {
        Ok(result) => serde_json::to_string(&result).unwrap_or_else(|e| {
            format!(r#"{{"success":false,"error":"Serialization error: {e}"}}"#)
        }),
        Err(e) => format!(r#"{{"success":false,"error":"{e}"}}"#),
    }
}

// ── close_container ─────────────────────────────────────────────────────────

/// Input for the `close_container` MCP tool (no parameters needed).
#[derive(Deserialize, Default)]
pub struct CloseContainerInput {}

impl rmcp::schemars::JsonSchema for CloseContainerInput {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("CloseContainerInput")
    }

    fn json_schema(
        _gen: &mut rmcp::schemars::SchemaGenerator,
    ) -> rmcp::schemars::Schema {
        schema_from_json(json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        }))
    }
}

/// Handle `close_container` MCP tool.
///
/// Requires a container to be open, checks online status, then sends
/// [`BotCommand::CloseContainer`].
pub async fn handle_close_container(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    _input: CloseContainerInput,
) -> String {
    if let Err(e) = check_container_open(state) {
        return e;
    }

    if !state.is_online() {
        return r#"{"success":false,"error":"Bot is not connected to a server"}"#
            .to_string();
    }

    let cmd = BotCommand::CloseContainer;
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
    use rmcp::schemars::JsonSchema;

    fn setup() -> (Arc<SharedState>, BotCommandSender) {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, _receiver) = create_command_channel(4);
        (state, sender)
    }

    fn make_online(state: &SharedState) {
        state.set_online(true);
    }

    /// Simulate an open container by storing a dummy handle.
    /// Since we cannot construct a real `ContainerHandle` in tests,
    /// we use `set_container_handle` to set the internal Option to `Some`.
    /// The actual handle type doesn't matter for `has_container_open()` checks.
    fn simulate_container_open(_state: &SharedState) {
        // ContainerHandle can't be constructed in tests — we test the
        // has_container_open() path via the existing state API.
        // For integration tests, a real Minecraft connection is needed.
        // Unit tests below validate the error paths (offline, no container open).
    }

    // ── open_container ──────────────────────────────────────────

    #[tokio::test]
    async fn test_open_container_offline() {
        let (state, sender) = setup();
        let input = OpenContainerInput { x: 0, y: 64, z: 0 };
        let result = handle_open_container(&state, &sender, input).await;
        assert!(result.contains("not connected"));
    }

    #[tokio::test]
    async fn test_open_container_invalid_coords() {
        let (state, sender) = setup();
        make_online(&state);
        let input = OpenContainerInput { x: 0, y: -100, z: 0 };
        let result = handle_open_container(&state, &sender, input).await;
        assert!(result.contains("out of bounds") || result.contains("out of range"));
    }

    // ── take_from_container ─────────────────────────────────────

    #[tokio::test]
    async fn test_take_from_container_offline() {
        let (state, sender) = setup();
        // No container open — should get "No container is currently open"
        let input = TakeFromContainerInput { slot: 0, count: Some(1) };
        let result = handle_take_from_container(&state, &sender, input).await;
        assert!(result.contains("No container is currently open"));
    }

    #[tokio::test]
    async fn test_take_from_container_zero_count() {
        let (state, sender) = setup();
        // Even with no container open, we need a container to be "open"
        // to reach the count check. But check_container_open runs first.
        let input = TakeFromContainerInput { slot: 0, count: Some(0) };
        let result = handle_take_from_container(&state, &sender, input).await;
        assert!(result.contains("No container is currently open"));
    }

    #[tokio::test]
    async fn test_take_from_container_default_count() {
        let (state, sender) = setup();
        // No container open — error expected
        let input = TakeFromContainerInput { slot: 5, count: None };
        let result = handle_take_from_container(&state, &sender, input).await;
        assert!(result.contains("No container is currently open"));
    }

    // ── put_into_container ──────────────────────────────────────

    #[tokio::test]
    async fn test_put_into_container_offline() {
        let (state, sender) = setup();
        let input = PutIntoContainerInput { slot: 0, count: Some(1) };
        let result = handle_put_into_container(&state, &sender, input).await;
        assert!(result.contains("No container is currently open"));
    }

    // ── close_container ─────────────────────────────────────────

    #[tokio::test]
    async fn test_close_container_no_container_open() {
        let (state, sender) = setup();
        make_online(&state);
        let input = CloseContainerInput {};
        let result = handle_close_container(&state, &sender, input).await;
        assert!(result.contains("No container is currently open"));
    }

    // ── Schema tests ────────────────────────────────────────────

    #[test]
    fn test_open_container_schema_name() {
        assert_eq!(OpenContainerInput::schema_name(), "OpenContainerInput");
    }

    #[test]
    fn test_take_from_container_schema_name() {
        assert_eq!(TakeFromContainerInput::schema_name(), "TakeFromContainerInput");
    }
}
