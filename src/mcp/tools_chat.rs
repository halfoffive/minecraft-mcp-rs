//! MCP tools for chat, commands, and game mode operations.
//!
//! Provides the implementation behind `send_chat`, `execute_command`, and
//! `set_game_mode` MCP tools. Each function validates input, constructs the
//! appropriate [`BotCommand`], and sends it through the command channel.

use serde::Deserialize;
use serde_json::json;

use crate::channel::BotCommandSender;
use crate::types::{BotCommand, GameMode};

// ---------------------------------------------------------------------------
// Parameter structs (used by rmcp #[tool] macro in server.rs)
//
// We implement rmcp::schemars::JsonSchema manually because the project pins
// schemars v0.8 for existing data types, but rmcp 1.7 depends on schemars
// v1.2.1.  Derive macros would reference the wrong crate version.
// ---------------------------------------------------------------------------

/// Input for the `send_chat` tool.
#[derive(Deserialize, Default)]
pub struct SendChatInput {
    /// The message to send to the server chat.
    pub message: String,
}

impl rmcp::schemars::JsonSchema for SendChatInput {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "SendChatInput".into()
    }

    fn json_schema(_: &mut rmcp::schemars::SchemaGenerator) -> rmcp::schemars::Schema {
        rmcp::schemars::Schema::from(
            json!({
                "type": "object",
                "properties": {
                    "message": {
                        "type": "string",
                        "description": "The message to send to the server chat"
                    }
                },
                "required": ["message"],
                "additionalProperties": false
            })
            .as_object()
            .unwrap()
            .clone(),
        )
    }
}

/// Input for the `execute_command` tool.
#[derive(Deserialize, Default)]
pub struct ExecuteCommandInput {
    /// The command to execute (with or without leading `/`).
    pub command: String,
}

impl rmcp::schemars::JsonSchema for ExecuteCommandInput {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "ExecuteCommandInput".into()
    }

    fn json_schema(_: &mut rmcp::schemars::SchemaGenerator) -> rmcp::schemars::Schema {
        rmcp::schemars::Schema::from(
            json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The command to execute (with or without leading /)"
                    }
                },
                "required": ["command"],
                "additionalProperties": false
            })
            .as_object()
            .unwrap()
            .clone(),
        )
    }
}

/// Input for the `set_game_mode` tool.
#[derive(Deserialize, Default)]
pub struct SetGameModeInput {
    /// The game mode to set. One of: survival, creative, adventure, spectator.
    pub mode: String,
}

impl rmcp::schemars::JsonSchema for SetGameModeInput {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "SetGameModeInput".into()
    }

    fn json_schema(_: &mut rmcp::schemars::SchemaGenerator) -> rmcp::schemars::Schema {
        rmcp::schemars::Schema::from(json!({
            "type": "object",
            "properties": {
                "mode": {
                    "type": "string",
                    "description": "Game mode to set. One of: survival, creative, adventure, spectator",
                    "enum": ["survival", "creative", "adventure", "spectator"]
                }
            },
            "required": ["mode"],
            "additionalProperties": false
        }).as_object().unwrap().clone())
    }
}

// ---------------------------------------------------------------------------
// Handler functions
// ---------------------------------------------------------------------------

/// Send a chat message to the server.
///
/// Validates the message is non-empty (whitespace-only is also rejected),
/// then sends [`BotCommand::SendChat`] through the command channel.
pub async fn handle_send_chat(sender: &BotCommandSender, message: String) -> String {
    if message.trim().is_empty() {
        return "Error: Message cannot be empty".to_string();
    }

    let cmd = BotCommand::SendChat(message);
    match sender.send_command(cmd).await {
        Ok(result) => result.message,
        Err(e) => format!("Error: {}", e),
    }
}

/// Execute a Minecraft command.
///
/// Validates the command is non-empty, auto-prepends `/` if it does not
/// already start with one, then sends [`BotCommand::ExecuteCommand`].
pub async fn handle_execute_command(sender: &BotCommandSender, command: String) -> String {
    if command.trim().is_empty() {
        return "Error: Command cannot be empty".to_string();
    }

    // Auto-prepend `/` if the user omitted it.
    let cmd_str = if command.starts_with('/') {
        command
    } else {
        format!("/{}", command)
    };

    let cmd = BotCommand::ExecuteCommand(cmd_str);
    match sender.send_command(cmd).await {
        Ok(result) => result.message,
        Err(e) => format!("Error: {}", e),
    }
}

/// Set the bot's game mode.
///
/// Validates the mode string (case-insensitive) is one of: survival, creative,
/// adventure, or spectator. Requires operator permissions on the server.
pub async fn handle_set_game_mode(sender: &BotCommandSender, mode: String) -> String {
    let game_mode = match mode.to_lowercase().as_str() {
        "survival" => GameMode::Survival,
        "creative" => GameMode::Creative,
        "adventure" => GameMode::Adventure,
        "spectator" => GameMode::Spectator,
        _ => {
            return format!(
                "Error: Invalid game mode '{mode}'. Must be one of: survival, creative, adventure, spectator"
            );
        }
    };

    let cmd = BotCommand::SetGameMode(game_mode);
    match sender.send_command(cmd).await {
        Ok(result) => result.message,
        Err(e) => format!("Error: {}", e),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::create_command_channel;
    use crate::types::BotResult;

    // -- Helpers --------------------------------------------------------------

    /// Returns `(sender, receiver)` with a small buffer. The receiver side
    /// immediately responds with a successful `BotResult` carrying the
    /// command's debug string as the message.
    fn make_echo_channel() -> (
        BotCommandSender,
        tokio::sync::mpsc::UnboundedReceiver<String>,
    ) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let (sender, mut receiver) = create_command_channel(10);

        // Spawn a responder that echoes the command name back
        tokio::spawn(async move {
            while let Some(wrapped) = receiver.recv().await {
                let cmd_debug = format!("{:?}", wrapped.command);
                let msg = format!("executed: {cmd_debug}");
                let _ = tx.send(cmd_debug.clone());
                let _ = wrapped.respond_to.send(Ok(BotResult {
                    success: true,
                    message: msg,
                    data: None,
                }));
            }
        });

        (sender, rx)
    }

    // -- send_chat ------------------------------------------------------------

    #[tokio::test]
    async fn test_handle_send_chat_valid() {
        let (sender, _rx) = make_echo_channel();
        let result = handle_send_chat(&sender, "hello".into()).await;
        assert!(!result.contains("Error"), "unexpected error: {result}");
    }

    #[tokio::test]
    async fn test_handle_send_chat_empty_rejected() {
        let (sender, _rx) = make_echo_channel();
        let result = handle_send_chat(&sender, "".into()).await;
        assert!(result.contains("Error"), "empty should be rejected");
        assert!(result.contains("empty"));
    }

    #[tokio::test]
    async fn test_handle_send_chat_whitespace_rejected() {
        let (sender, _rx) = make_echo_channel();
        let result = handle_send_chat(&sender, "   ".into()).await;
        assert!(result.contains("Error"), "whitespace should be rejected");
    }

    // -- execute_command ------------------------------------------------------

    #[tokio::test]
    async fn test_handle_execute_command_valid() {
        let (sender, mut rx) = make_echo_channel();
        let _ = handle_execute_command(&sender, "gamemode creative".into()).await;

        // The command should have been auto-prepended with /
        let sent = rx.recv().await.expect("should receive command");
        assert!(
            sent.contains("ExecuteCommand(\"/gamemode creative\""),
            "expected command with auto-prepended /, got: {sent}"
        );
    }

    #[tokio::test]
    async fn test_handle_execute_command_with_slash() {
        let (sender, mut rx) = make_echo_channel();
        let _ = handle_execute_command(&sender, "/gamemode creative".into()).await;

        let sent = rx.recv().await.expect("should receive command");
        assert!(
            sent.contains("ExecuteCommand(\"/gamemode creative\""),
            "expected command with existing /, got: {sent}"
        );
    }

    #[tokio::test]
    async fn test_handle_execute_command_empty_rejected() {
        let (sender, _rx) = make_echo_channel();
        let result = handle_execute_command(&sender, "".into()).await;
        assert!(result.contains("Error"));
    }

    // -- set_game_mode --------------------------------------------------------

    #[tokio::test]
    async fn test_handle_set_game_mode_survival() {
        let (sender, mut rx) = make_echo_channel();
        let result = handle_set_game_mode(&sender, "survival".into()).await;
        assert!(!result.contains("Error"), "unexpected error: {result}");

        let sent = rx.recv().await.expect("should receive command");
        assert!(sent.contains("SetGameMode(Survival)"));
    }

    #[tokio::test]
    async fn test_handle_set_game_mode_creative() {
        let (sender, mut rx) = make_echo_channel();
        let _ = handle_set_game_mode(&sender, "creative".into()).await;

        let sent = rx.recv().await.expect("should receive command");
        assert!(sent.contains("SetGameMode(Creative)"));
    }

    #[tokio::test]
    async fn test_handle_set_game_mode_adventure() {
        let (sender, mut rx) = make_echo_channel();
        let _ = handle_set_game_mode(&sender, "adventure".into()).await;

        let sent = rx.recv().await.expect("should receive command");
        assert!(sent.contains("SetGameMode(Adventure)"));
    }

    #[tokio::test]
    async fn test_handle_set_game_mode_spectator() {
        let (sender, mut rx) = make_echo_channel();
        let _ = handle_set_game_mode(&sender, "spectator".into()).await;

        let sent = rx.recv().await.expect("should receive command");
        assert!(sent.contains("SetGameMode(Spectator)"));
    }

    #[tokio::test]
    async fn test_handle_set_game_mode_case_insensitive() {
        let (sender, mut rx) = make_echo_channel();
        let _ = handle_set_game_mode(&sender, "Creative".into()).await;

        let sent = rx.recv().await.expect("should receive command");
        assert!(sent.contains("SetGameMode(Creative)"));
    }

    #[tokio::test]
    async fn test_handle_set_game_mode_invalid_mode() {
        let (sender, _rx) = make_echo_channel();
        let result = handle_set_game_mode(&sender, "invalid".into()).await;
        assert!(result.contains("Error"));
        assert!(result.contains("Invalid game mode"));
        assert!(result.contains("invalid"));
    }

    #[tokio::test]
    async fn test_handle_set_game_mode_empty_rejected() {
        let (sender, _rx) = make_echo_channel();
        let result = handle_set_game_mode(&sender, "".into()).await;
        assert!(result.contains("Error"));
    }
}
