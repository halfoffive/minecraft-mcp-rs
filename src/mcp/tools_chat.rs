//! MCP tools for chat, commands, and game mode operations.
//!
//! Provides the implementation behind `send_chat`, `execute_command`,
//! `set_game_mode`, and `get_chat_history` MCP tools. Each function validates
//! input, constructs the appropriate [`BotCommand`], and sends it through the
//! command channel (or reads directly from [`SharedState`] for queries).

use std::sync::Arc;

use serde::Deserialize;
use serde_json::json;

use crate::channel::BotCommandSender;
use crate::error::BotError;
use crate::state::SharedState;
use crate::types::{BotCommand, GameMode};

// ---------------------------------------------------------------------------
// Parameter structs (used by rmcp #[tool] macro in server.rs)
// ---------------------------------------------------------------------------

/// Input for the `send_chat` tool.
#[derive(Deserialize, Default, rmcp::schemars::JsonSchema)]
pub struct SendChatInput {
    /// The message to send to the server chat.
    pub message: String,
}

/// Input for the `execute_command` tool.
#[derive(Deserialize, Default, rmcp::schemars::JsonSchema)]
pub struct ExecuteCommandInput {
    /// The command to execute (with or without leading `/`).
    pub command: String,
}

/// Input for the `set_game_mode` tool.
#[derive(Deserialize, Default, rmcp::schemars::JsonSchema)]
pub struct SetGameModeInput {
    /// The game mode to set. One of: survival, creative, adventure, spectator.
    pub mode: String,
}

// ---------------------------------------------------------------------------
// Handler functions
// ---------------------------------------------------------------------------

/// Send a chat message to the server.
///
/// Returns an offline error if the bot is not connected. Otherwise validates
/// the message is non-empty (whitespace-only is also rejected), then sends
/// [`BotCommand::SendChat`] through the command channel.
pub async fn handle_send_chat(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    message: String,
) -> Result<String, BotError> {
    if !state.is_online() {
        return Err(BotError::Offline("Bot is offline".to_string()));
    }

    if message.trim().is_empty() {
        return Err(BotError::InvalidParams(
            "Message cannot be empty".to_string(),
        ));
    }

    let cmd = BotCommand::SendChat(message);
    match sender.send_command(cmd).await {
        Ok(result) => Ok(result.message),
        Err(e) => Err(BotError::Internal(format!("Command failed: {e}"))),
    }
}

/// Execute a Minecraft command.
///
/// Returns an offline error if the bot is not connected. Otherwise validates
/// the command is non-empty, auto-prepends `/` if it does not already start
/// with one, then sends [`BotCommand::ExecuteCommand`].
pub async fn handle_execute_command(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    command: String,
) -> Result<String, BotError> {
    if !state.is_online() {
        return Err(BotError::Offline("Bot is offline".to_string()));
    }

    if command.trim().is_empty() {
        return Err(BotError::InvalidParams(
            "Command cannot be empty".to_string(),
        ));
    }

    // Auto-prepend `/` if the user omitted it.
    let cmd_str = if command.starts_with('/') {
        command
    } else {
        format!("/{}", command)
    };

    let cmd = BotCommand::ExecuteCommand(cmd_str);
    match sender.send_command(cmd).await {
        Ok(result) => Ok(result.message),
        Err(e) => Err(BotError::Internal(format!("Command failed: {e}"))),
    }
}

/// Set the bot's game mode.
///
/// Returns an offline error if the bot is not connected. Otherwise validates
/// the mode string (case-insensitive) is one of: survival, creative,
/// adventure, or spectator. Requires operator permissions on the server.
pub async fn handle_set_game_mode(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    mode: String,
) -> Result<String, BotError> {
    if !state.is_online() {
        return Err(BotError::Offline("Bot is offline".to_string()));
    }

    let game_mode = match mode.to_lowercase().as_str() {
        "survival" => GameMode::Survival,
        "creative" => GameMode::Creative,
        "adventure" => GameMode::Adventure,
        "spectator" => GameMode::Spectator,
        _ => {
            return Err(BotError::InvalidParams(format!(
                "Invalid game mode '{mode}'. Must be one of: survival, creative, adventure, spectator"
            )));
        }
    };

    let cmd = BotCommand::SetGameMode(game_mode);
    match sender.send_command(cmd).await {
        Ok(result) => Ok(result.message),
        Err(e) => Err(BotError::Internal(format!("Command failed: {e}"))),
    }
}

// ---------------------------------------------------------------------------
// get_chat_history — reads recent chat messages from SharedState
// ---------------------------------------------------------------------------

/// Return recent chat messages (up to 10) as a JSON array.
///
/// Each entry is an object `{"sender":"...","message":"..."}`. Returns an
/// error when the bot is not connected to a server.
pub fn get_chat_history(state: &Arc<SharedState>) -> Result<String, BotError> {
    if !state.is_online() {
        return Err(BotError::Offline("Bot is offline".to_string()));
    }

    let messages = state.get_chat_messages();
    let entries: Vec<_> = messages
        .into_iter()
        .map(|(sender, message)| {
            json!({
                "sender": sender,
                "message": message,
            })
        })
        .collect();

    serde_json::to_string(&entries)
        .map_err(|e| BotError::Internal(format!("Serialization error: {e}")))
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
        let state = make_state(true);
        let result = handle_send_chat(&state, &sender, "hello".into()).await;
        assert!(
            !result.as_ref().unwrap().contains("Error"),
            "unexpected error: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_handle_send_chat_empty_rejected() {
        let (sender, _rx) = make_echo_channel();
        let state = make_state(true);
        let result = handle_send_chat(&state, &sender, "".into()).await;
        assert!(
            matches!(result, Err(BotError::InvalidParams(ref msg)) if msg.contains("empty")),
            "empty should be rejected"
        );
    }

    #[tokio::test]
    async fn test_handle_send_chat_whitespace_rejected() {
        let (sender, _rx) = make_echo_channel();
        let state = make_state(true);
        let result = handle_send_chat(&state, &sender, "   ".into()).await;
        assert!(
            matches!(result, Err(BotError::InvalidParams(ref msg)) if msg.contains("empty")),
            "whitespace should be rejected"
        );
    }

    #[tokio::test]
    async fn test_handle_send_chat_offline() {
        let (sender, _rx) = make_echo_channel();
        let state = make_state(false);
        let result = handle_send_chat(&state, &sender, "hello".into()).await;
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    // -- execute_command ------------------------------------------------------

    #[tokio::test]
    async fn test_handle_execute_command_valid() {
        let (sender, mut rx) = make_echo_channel();
        let state = make_state(true);
        let _ = handle_execute_command(&state, &sender, "gamemode creative".into()).await;

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
        let state = make_state(true);
        let _ = handle_execute_command(&state, &sender, "/gamemode creative".into()).await;

        let sent = rx.recv().await.expect("should receive command");
        assert!(
            sent.contains("ExecuteCommand(\"/gamemode creative\""),
            "expected command with existing /, got: {sent}"
        );
    }

    #[tokio::test]
    async fn test_handle_execute_command_empty_rejected() {
        let (sender, _rx) = make_echo_channel();
        let state = make_state(true);
        let result = handle_execute_command(&state, &sender, "".into()).await;
        assert!(matches!(result, Err(BotError::InvalidParams(_))));
    }

    #[tokio::test]
    async fn test_handle_execute_command_offline() {
        let (sender, _rx) = make_echo_channel();
        let state = make_state(false);
        let result = handle_execute_command(&state, &sender, "gamemode creative".into()).await;
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    // -- set_game_mode --------------------------------------------------------

    #[tokio::test]
    async fn test_handle_set_game_mode_survival() {
        let (sender, mut rx) = make_echo_channel();
        let state = make_state(true);
        let result = handle_set_game_mode(&state, &sender, "survival".into()).await;
        assert!(
            !result.as_ref().unwrap().contains("Error"),
            "unexpected error: {result:?}"
        );

        let sent = rx.recv().await.expect("should receive command");
        assert!(sent.contains("SetGameMode(Survival)"));
    }

    #[tokio::test]
    async fn test_handle_set_game_mode_creative() {
        let (sender, mut rx) = make_echo_channel();
        let state = make_state(true);
        let _ = handle_set_game_mode(&state, &sender, "creative".into()).await;

        let sent = rx.recv().await.expect("should receive command");
        assert!(sent.contains("SetGameMode(Creative)"));
    }

    #[tokio::test]
    async fn test_handle_set_game_mode_adventure() {
        let (sender, mut rx) = make_echo_channel();
        let state = make_state(true);
        let _ = handle_set_game_mode(&state, &sender, "adventure".into()).await;

        let sent = rx.recv().await.expect("should receive command");
        assert!(sent.contains("SetGameMode(Adventure)"));
    }

    #[tokio::test]
    async fn test_handle_set_game_mode_spectator() {
        let (sender, mut rx) = make_echo_channel();
        let state = make_state(true);
        let _ = handle_set_game_mode(&state, &sender, "spectator".into()).await;

        let sent = rx.recv().await.expect("should receive command");
        assert!(sent.contains("SetGameMode(Spectator)"));
    }

    #[tokio::test]
    async fn test_handle_set_game_mode_case_insensitive() {
        let (sender, mut rx) = make_echo_channel();
        let state = make_state(true);
        let _ = handle_set_game_mode(&state, &sender, "Creative".into()).await;

        let sent = rx.recv().await.expect("should receive command");
        assert!(sent.contains("SetGameMode(Creative)"));
    }

    #[tokio::test]
    async fn test_handle_set_game_mode_invalid_mode() {
        let (sender, _rx) = make_echo_channel();
        let state = make_state(true);
        let result = handle_set_game_mode(&state, &sender, "invalid".into()).await;
        assert!(matches!(result, Err(BotError::InvalidParams(ref msg))
                if msg.contains("Invalid game mode") && msg.contains("invalid")));
    }

    #[tokio::test]
    async fn test_handle_set_game_mode_empty_rejected() {
        let (sender, _rx) = make_echo_channel();
        let state = make_state(true);
        let result = handle_set_game_mode(&state, &sender, "".into()).await;
        assert!(matches!(result, Err(BotError::InvalidParams(_))));
    }

    #[tokio::test]
    async fn test_handle_set_game_mode_offline() {
        let (sender, _rx) = make_echo_channel();
        let state = make_state(false);
        let result = handle_set_game_mode(&state, &sender, "creative".into()).await;
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    // -- get_chat_history -----------------------------------------------------

    fn make_state(online: bool) -> Arc<SharedState> {
        let state = Arc::new(SharedState::new(crate::config::AppConfig::default()));
        state.set_online(online);
        state
    }

    #[test]
    fn test_get_chat_history_online() {
        let state = make_state(true);
        state.add_chat_message("Alice".into(), "Hello".into());
        state.add_chat_message("Bob".into(), "Hi there".into());

        let result = get_chat_history(&state).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).expect("valid JSON");
        let arr = parsed
            .as_array()
            .expect("expected a JSON array of messages");
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["sender"], "Alice");
        assert_eq!(arr[0]["message"], "Hello");
        assert_eq!(arr[1]["sender"], "Bob");
        assert_eq!(arr[1]["message"], "Hi there");
    }

    #[test]
    fn test_get_chat_history_online_empty() {
        let state = make_state(true);
        let result = get_chat_history(&state).unwrap();
        assert_eq!(result, "[]");
    }

    #[test]
    fn test_get_chat_history_offline() {
        let state = make_state(false);
        state.add_chat_message("Alice".into(), "Hello".into());
        let result = get_chat_history(&state);
        assert!(matches!(result, Err(BotError::Offline(_))));
    }
}
