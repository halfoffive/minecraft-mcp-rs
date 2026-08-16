//! MCP tools for container interaction (chests, furnaces, etc.).
//!
//! Each tool validates parameters, checks online status and container state,
//! and dispatches a [`BotCommand`] through the bot command channel.

use std::sync::Arc;

use serde::Deserialize;

use crate::channel::BotCommandSender;
use crate::command_validate::validate_block_pos;
use crate::error::BotError;
use crate::state::SharedState;
use crate::types::{BlockPos, BotCommand};

// ── Container state helpers ─────────────────────────────────────────────────

/// Ensure a container is currently open, returning a [`BotError`] otherwise.
fn check_container_open(state: &SharedState) -> Result<(), BotError> {
    if !state.has_container_open() {
        return Err(BotError::InvalidParams(
            "No container is currently open".to_string(),
        ));
    }
    Ok(())
}

/// Ensure no container is currently open, returning a [`BotError`] otherwise.
fn check_container_not_open(state: &SharedState) -> Result<(), BotError> {
    if state.has_container_open() {
        return Err(BotError::ContainerAlreadyOpen);
    }
    Ok(())
}

// ── open_container ──────────────────────────────────────────────────────────

/// Input for the `open_container` MCP tool.
#[derive(Deserialize, Default, rmcp::schemars::JsonSchema)]
pub struct OpenContainerInput {
    /// X coordinate of the container to open.
    pub x: i32,
    /// Y coordinate of the container to open.
    pub y: i32,
    /// Z coordinate of the container to open.
    pub z: i32,
}

/// Handle `open_container` MCP tool.
///
/// Validates coordinates, ensures no container is already open,
/// checks online status, then sends [`BotCommand::OpenContainer`].
pub async fn handle_open_container(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    input: OpenContainerInput,
) -> Result<String, BotError> {
    if let Err(e) = validate_block_pos(&BlockPos::new(input.x, input.y, input.z)) {
        return Err(BotError::InvalidParams(e));
    }

    if !state.is_online() {
        return Err(BotError::Offline(
            "Bot is not connected to a server".to_string(),
        ));
    }

    check_container_not_open(state)?;

    let cmd = BotCommand::OpenContainer(BlockPos::new(input.x, input.y, input.z));
    match sender.send_command(cmd).await {
        Ok(result) => serde_json::to_string(&result)
            .map_err(|e| BotError::Internal(format!("Serialization error: {e}"))),
        Err(e) => Err(e),
    }
}

// ── take_from_container ─────────────────────────────────────────────────────

/// Input for the `take_from_container` MCP tool.
#[derive(Deserialize, Default, rmcp::schemars::JsonSchema)]
pub struct TakeFromContainerInput {
    /// Container slot index (0-based) to take items from. Double chests
    /// have 54 slots (0-53).
    #[schemars(range(min = 0, max = 53))]
    pub slot: u8,
    /// Number of items to take (default 1). NOTE: this is a HINT — the
    /// underlying container click moves the whole stack (shift-click), so
    /// the actual amount transferred may exceed `count`.
    #[schemars(range(min = 1, max = 64))]
    pub count: Option<u8>,
}

/// Handle `take_from_container` MCP tool.
///
/// Requires a container to be open, checks online status, then sends
/// [`BotCommand::TakeFromContainer`].
pub async fn handle_take_from_container(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    input: TakeFromContainerInput,
) -> Result<String, BotError> {
    // Validate parameters first: malformed parameters should be reported as
    // InvalidParams regardless of whether a container is open or the bot
    // is online.  This matches the convention in tools_item.rs.
    if input.slot > 53 {
        return Err(BotError::InvalidParams(format!(
            "Container slot must be 0-53, got {}",
            input.slot
        )));
    }
    let count = input.count.unwrap_or(1);
    if !(1..=64).contains(&count) {
        return Err(BotError::InvalidParams(format!(
            "Count must be between 1 and 64, got {count}"
        )));
    }

    if !state.is_online() {
        return Err(BotError::Offline(
            "Bot is not connected to a server".to_string(),
        ));
    }

    check_container_open(state)?;

    let cmd = BotCommand::TakeFromContainer(input.slot, count);
    match sender.send_command(cmd).await {
        Ok(result) => serde_json::to_string(&result)
            .map_err(|e| BotError::Internal(format!("Serialization error: {e}"))),
        Err(e) => Err(e),
    }
}

// ── put_into_container ──────────────────────────────────────────────────────

/// Input for the `put_into_container` MCP tool.
#[derive(Deserialize, Default, rmcp::schemars::JsonSchema)]
pub struct PutIntoContainerInput {
    /// Container slot index (0-based) to put items into. Double chests
    /// have 54 slots (0-53).
    #[schemars(range(min = 0, max = 53))]
    pub slot: u8,
    /// Number of items to put (default 1). NOTE: this is a HINT — the
    /// underlying container click moves the whole stack (shift-click), so
    /// the actual amount transferred may exceed `count`.
    #[schemars(range(min = 1, max = 64))]
    pub count: Option<u8>,
}

/// Handle `put_into_container` MCP tool.
///
/// Requires a container to be open, checks online status, then sends
/// [`BotCommand::PutIntoContainer`].
pub async fn handle_put_into_container(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    input: PutIntoContainerInput,
) -> Result<String, BotError> {
    // Validate parameters first: same reasoning as handle_take_from_container.
    if input.slot > 53 {
        return Err(BotError::InvalidParams(format!(
            "Container slot must be 0-53, got {}",
            input.slot
        )));
    }
    let count = input.count.unwrap_or(1);
    if !(1..=64).contains(&count) {
        return Err(BotError::InvalidParams(format!(
            "Count must be between 1 and 64, got {count}"
        )));
    }

    if !state.is_online() {
        return Err(BotError::Offline(
            "Bot is not connected to a server".to_string(),
        ));
    }

    check_container_open(state)?;

    let cmd = BotCommand::PutIntoContainer(input.slot, count);
    match sender.send_command(cmd).await {
        Ok(result) => serde_json::to_string(&result)
            .map_err(|e| BotError::Internal(format!("Serialization error: {e}"))),
        Err(e) => Err(e),
    }
}

// ── close_container ─────────────────────────────────────────────────────────

/// Input for the `close_container` MCP tool (no parameters needed).
#[derive(Deserialize, Default, rmcp::schemars::JsonSchema)]
pub struct CloseContainerInput {}

/// Handle `close_container` MCP tool.
///
/// Requires a container to be open, checks online status, then sends
/// [`BotCommand::CloseContainer`].
pub async fn handle_close_container(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    _input: CloseContainerInput,
) -> Result<String, BotError> {
    if !state.is_online() {
        return Err(BotError::Offline(
            "Bot is not connected to a server".to_string(),
        ));
    }

    check_container_open(state)?;

    let cmd = BotCommand::CloseContainer;
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
    use rmcp::schemars::JsonSchema;

    fn setup() -> (Arc<SharedState>, BotCommandSender) {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, _receiver) = create_command_channel(4, Arc::clone(&state));
        (state, sender)
    }

    fn make_online(state: &SharedState) {
        state.set_online(true);
    }

    // ── open_container ──────────────────────────────────────────

    #[tokio::test]
    async fn test_open_container_offline() {
        let (state, sender) = setup();
        let input = OpenContainerInput { x: 0, y: 64, z: 0 };
        let result = handle_open_container(&state, &sender, input).await;
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    #[tokio::test]
    async fn test_open_container_invalid_coords() {
        let (state, sender) = setup();
        make_online(&state);
        let input = OpenContainerInput {
            x: 0,
            y: -100,
            z: 0,
        };
        let result = handle_open_container(&state, &sender, input).await;
        assert!(matches!(result, Err(BotError::InvalidParams(ref msg))
                if msg.contains("out of bounds") || msg.contains("out of range")));
    }

    // ── take_from_container ─────────────────────────────────────

    #[tokio::test]
    async fn test_take_from_container_offline() {
        let (state, sender) = setup();
        // Parameters are valid, but bot is offline — should get Offline error
        // (offline check comes before container-open check)
        let input = TakeFromContainerInput {
            slot: 0,
            count: Some(1),
        };
        let result = handle_take_from_container(&state, &sender, input).await;
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    #[tokio::test]
    async fn test_take_from_container_zero_count() {
        let (state, sender) = setup();
        // With the new validation order, count==0 is rejected before
        // container-open checks, so we get a count-specific error even
        // without a container being open.
        let input = TakeFromContainerInput {
            slot: 0,
            count: Some(0),
        };
        let result = handle_take_from_container(&state, &sender, input).await;
        assert!(
            matches!(result, Err(BotError::InvalidParams(ref msg))
                if msg.contains("Count must be between 1 and 64")),
            "expected count==0 error, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_take_from_container_default_count() {
        let (state, sender) = setup();
        // Default count=1 is valid, but bot is offline — should get Offline error
        let input = TakeFromContainerInput {
            slot: 5,
            count: None,
        };
        let result = handle_take_from_container(&state, &sender, input).await;
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    // ── put_into_container ──────────────────────────────────────

    #[tokio::test]
    async fn test_put_into_container_offline() {
        let (state, sender) = setup();
        let input = PutIntoContainerInput {
            slot: 0,
            count: Some(1),
        };
        // Parameters valid but bot offline — Offline error expected
        let result = handle_put_into_container(&state, &sender, input).await;
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    // ── close_container ─────────────────────────────────────────

    #[tokio::test]
    async fn test_close_container_no_container_open() {
        let (state, sender) = setup();
        make_online(&state);
        let input = CloseContainerInput {};
        let result = handle_close_container(&state, &sender, input).await;
        assert!(matches!(result, Err(BotError::InvalidParams(ref msg))
                if msg.contains("No container is currently open")));
    }

    // ── Schema tests ────────────────────────────────────────────

    #[test]
    fn test_open_container_schema_name() {
        assert_eq!(OpenContainerInput::schema_name(), "OpenContainerInput");
    }

    #[test]
    fn test_take_from_container_schema_name() {
        assert_eq!(
            TakeFromContainerInput::schema_name(),
            "TakeFromContainerInput"
        );
    }

    /// Verifies the JSON Schema published to MCP clients advertises a slot
    /// max of 53 (not the old 50). The schemars `range` attribute should
    /// surface as a `maximum: 53` constraint on the `slot` integer.
    #[test]
    fn test_take_from_container_schema_slot_max_is_53() {
        let schema = rmcp::schemars::schema_for!(TakeFromContainerInput);
        let value = serde_json::to_value(&schema).expect("schema serialises");
        let slot_schema = value
            .get("properties")
            .and_then(|p| p.get("slot"))
            .expect("schema has properties.slot");
        assert_eq!(
            slot_schema.get("maximum").and_then(|m| m.as_u64()),
            Some(53),
            "TakeFromContainerInput.slot must advertise maximum=53, got {slot_schema}"
        );
        assert_eq!(
            slot_schema.get("minimum").and_then(|m| m.as_u64()),
            Some(0),
            "TakeFromContainerInput.slot must advertise minimum=0, got {slot_schema}"
        );
    }

    #[test]
    fn test_put_into_container_schema_slot_max_is_53() {
        let schema = rmcp::schemars::schema_for!(PutIntoContainerInput);
        let value = serde_json::to_value(&schema).expect("schema serialises");
        let slot_schema = value
            .get("properties")
            .and_then(|p| p.get("slot"))
            .expect("schema has properties.slot");
        assert_eq!(
            slot_schema.get("maximum").and_then(|m| m.as_u64()),
            Some(53),
            "PutIntoContainerInput.slot must advertise maximum=53, got {slot_schema}"
        );
    }
}
