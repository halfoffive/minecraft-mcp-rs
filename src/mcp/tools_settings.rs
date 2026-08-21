//! Settings & lifecycle MCP tools.
//!
//! Four tools that let an MCP client read and change the server's
//! configuration and drive the bot connection — all of them work while the
//! bot is OFFLINE (a client must be able to change the Minecraft server
//! address before the first connect):
//!
//! - `get_settings` — current config (token redacted) + runtime status.
//! - `update_settings` — partial config update; validated, then applied in
//!   memory (no file persistence — `MINECRAFT_MCP_*` env vars are the
//!   configuration source). Changing
//!   `mc_address`/`mc_port`/`ai_username` triggers a reconnect when the bot
//!   is online/connecting; `mcp_transport`/`mcp_address`/`mcp_port` take
//!   effect on process restart.
//! - `connect_bot` — spawn the bot connection (headless; the UI Connect
//!   button is the UI-mode equivalent).
//! - `disconnect_bot` — request a disconnect.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::channel::{BotCommandSender, ReceiverSlot};
#[cfg(test)]
use crate::config::AppConfig;
use crate::config::McpTransport;
use crate::error::BotError;
use crate::i18n::Language;
use crate::state::{McpServerStatus, SharedState};

// ── get_settings ───────────────────────────────────────────────────────────

/// Redacted view of [`AppConfig`](crate::config::AppConfig) for the
/// `get_settings` response.
///
/// [`AppConfig`](crate::config::AppConfig) must never be serialized
/// directly on public surfaces: `mcp_token` is a bearer credential and is
/// always replaced with `"***"` here.
#[derive(Serialize)]
struct SettingsView {
    mc_address: String,
    mc_port: u16,
    ai_username: String,
    mcp_address: String,
    mcp_port: u16,
    task_name: String,
    chunk_scan_radius: u8,
    block_perception_radius: u8,
    snapshot_interval_ms: u64,
    reconnect_initial_delay_ms: u64,
    reconnect_max_delay_ms: u64,
    command_timeout_secs: u64,
    fly_timeout_secs: u64,
    mcp_token: String,
    mcp_transport: String,
    mcp_auth_enabled: bool,
    language: String,
}

/// Human-readable one-line form of [`McpServerStatus`] for the
/// `get_settings` runtime block.
fn mcp_server_status_string(status: McpServerStatus) -> String {
    match status {
        McpServerStatus::Running(addr) => format!("running ({addr})"),
        McpServerStatus::Stdio => "stdio".to_string(),
        McpServerStatus::Failed(msg) => format!("failed: {msg}"),
        McpServerStatus::Stopped => "stopped".to_string(),
    }
}

/// Handle the `get_settings` MCP tool.
///
/// Returns pretty-printed JSON with the full configuration (the MCP token
/// always redacted to `"***"`) plus a runtime block: `online`,
/// `connecting` and `mcp_server_status`. Works offline.
pub fn get_settings(state: &Arc<SharedState>) -> Result<String, BotError> {
    let config = state.read_config().clone();
    let view = SettingsView {
        mc_address: config.mc_address,
        mc_port: config.mc_port,
        ai_username: config.ai_username,
        mcp_address: config.mcp_address,
        mcp_port: config.mcp_port,
        task_name: config.task_name,
        chunk_scan_radius: config.chunk_scan_radius,
        block_perception_radius: config.block_perception_radius,
        snapshot_interval_ms: config.snapshot_interval_ms,
        reconnect_initial_delay_ms: config.reconnect_initial_delay_ms,
        reconnect_max_delay_ms: config.reconnect_max_delay_ms,
        command_timeout_secs: config.command_timeout_secs,
        fly_timeout_secs: config.fly_timeout_secs,
        // NEVER expose the real token on a public surface.
        mcp_token: "***".to_string(),
        mcp_transport: transport_to_str(config.mcp_transport).to_string(),
        mcp_auth_enabled: config.mcp_auth_enabled,
        language: language_to_str(config.language).to_string(),
    };

    let mut value = serde_json::to_value(&view)
        .map_err(|e| BotError::Internal(format!("failed to serialize settings: {e}")))?;
    let obj = value
        .as_object_mut()
        .ok_or_else(|| BotError::Internal("settings view did not serialize to an object".into()))?;
    obj.insert("online".into(), json!(state.is_online()));
    obj.insert("connecting".into(), json!(state.is_connecting()));
    obj.insert(
        "mcp_server_status".into(),
        json!(mcp_server_status_string(state.get_mcp_server_status())),
    );

    serde_json::to_string_pretty(&value)
        .map_err(|e| BotError::Internal(format!("failed to serialize settings: {e}")))
}

// ── update_settings ────────────────────────────────────────────────────────

/// Input for the `update_settings` MCP tool.
///
/// All fields are optional — only the provided fields change (partial
/// update). Values are validated via [`AppConfig::validate`] and applied in
/// memory for the running process only — restart with `MINECRAFT_MCP_*`
/// environment variables to persist across restarts.
#[derive(Deserialize, Default, rmcp::schemars::JsonSchema)]
pub struct UpdateSettingsInput {
    /// Minecraft server address the bot connects to.
    pub mc_address: Option<String>,
    /// Minecraft server port (1-65535).
    #[schemars(range(min = 1))]
    pub mc_port: Option<u16>,
    /// Bot in-game username.
    pub ai_username: Option<String>,
    /// MCP server bind address (an IP or "localhost"). Takes effect on
    /// process restart.
    pub mcp_address: Option<String>,
    /// MCP server bind port (1-65535). Takes effect on process restart.
    #[schemars(range(min = 1))]
    pub mcp_port: Option<u16>,
    /// Descriptive task name shown in the UI.
    pub task_name: Option<String>,
    /// Chunks scanned around the player per snapshot (1-16).
    #[schemars(range(min = 1, max = 16))]
    pub chunk_scan_radius: Option<u8>,
    /// Block perception radius in blocks (8-64).
    #[schemars(range(min = 8, max = 64))]
    pub block_perception_radius: Option<u8>,
    /// Interval between world snapshots in milliseconds (> 0).
    pub snapshot_interval_ms: Option<u64>,
    /// Initial reconnect backoff delay in milliseconds (> 0).
    pub reconnect_initial_delay_ms: Option<u64>,
    /// Maximum reconnect backoff delay in milliseconds (>= initial).
    pub reconnect_max_delay_ms: Option<u64>,
    /// Timeout for bot commands in seconds (> 0).
    #[schemars(range(min = 1))]
    pub command_timeout_secs: Option<u64>,
    /// Timeout for `fly_to` long-distance flights in seconds (> 0, default
    /// 60). Independent of `command_timeout_secs` so long flights can
    /// breathe.
    #[schemars(range(min = 1))]
    pub fly_timeout_secs: Option<u64>,
    /// Bearer token MCP clients must present over HTTP. Redacted in every
    /// tool response; never persisted anywhere.
    pub mcp_token: Option<String>,
    /// MCP transport: "stdio" or "http". Takes effect on process restart.
    pub mcp_transport: Option<String>,
    /// Require a Bearer token over HTTP (default: false).
    pub mcp_auth_enabled: Option<bool>,
    /// UI language: "en" or "zh_cn".
    pub language: Option<String>,
}

/// Parse the `mcp_transport` input string into [`McpTransport`].
fn parse_transport(value: &str) -> Result<McpTransport, BotError> {
    if value.eq_ignore_ascii_case("stdio") {
        Ok(McpTransport::Stdio)
    } else if value.eq_ignore_ascii_case("http") {
        Ok(McpTransport::Http)
    } else {
        Err(BotError::InvalidParams(format!(
            "mcp_transport must be \"stdio\" or \"http\", got {value:?}"
        )))
    }
}

/// Parse the `language` input string into [`Language`].
fn parse_language(value: &str) -> Result<Language, BotError> {
    if value.eq_ignore_ascii_case("en") {
        Ok(Language::En)
    } else if value.eq_ignore_ascii_case("zh_cn") {
        Ok(Language::ZhCn)
    } else {
        Err(BotError::InvalidParams(format!(
            "language must be \"en\" or \"zh_cn\", got {value:?}"
        )))
    }
}

/// Serialize a transport back to its canonical lowercase input form.
fn transport_to_str(transport: McpTransport) -> &'static str {
    match transport {
        McpTransport::Stdio => "stdio",
        McpTransport::Http => "http",
    }
}

/// Serialize a language back to its canonical input form.
fn language_to_str(language: Language) -> &'static str {
    match language {
        Language::En => "en",
        Language::ZhCn => "zh_cn",
    }
}

/// Handle the `update_settings` MCP tool.
///
/// Partial update: only the provided fields change. The candidate config is
/// validated ([`AppConfig::validate`]) and applied in memory (no file
/// persistence). Changing `mc_address`/`mc_port`/`ai_username` triggers a
/// bot reconnect when connected/connecting; changes to
/// `mcp_transport`/`mcp_address`/`mcp_port` take effect on process restart.
/// Works offline.
pub fn update_settings(
    state: &Arc<SharedState>,
    input: UpdateSettingsInput,
) -> Result<String, BotError> {
    let old = state.read_config().clone();
    let mut candidate = old.clone();
    let mut applied = serde_json::Map::new();

    if let Some(v) = input.mc_address
        && v != candidate.mc_address
    {
        candidate.mc_address = v.clone();
        applied.insert("mc_address".into(), json!(v));
    }
    if let Some(v) = input.mc_port
        && v != candidate.mc_port
    {
        candidate.mc_port = v;
        applied.insert("mc_port".into(), json!(v));
    }
    if let Some(v) = input.ai_username
        && v != candidate.ai_username
    {
        candidate.ai_username = v.clone();
        applied.insert("ai_username".into(), json!(v));
    }
    if let Some(v) = input.mcp_address
        && v != candidate.mcp_address
    {
        candidate.mcp_address = v.clone();
        applied.insert("mcp_address".into(), json!(v));
    }
    if let Some(v) = input.mcp_port
        && v != candidate.mcp_port
    {
        candidate.mcp_port = v;
        applied.insert("mcp_port".into(), json!(v));
    }
    if let Some(v) = input.task_name
        && v != candidate.task_name
    {
        candidate.task_name = v.clone();
        applied.insert("task_name".into(), json!(v));
    }
    if let Some(v) = input.chunk_scan_radius
        && v != candidate.chunk_scan_radius
    {
        candidate.chunk_scan_radius = v;
        applied.insert("chunk_scan_radius".into(), json!(v));
    }
    if let Some(v) = input.block_perception_radius
        && v != candidate.block_perception_radius
    {
        candidate.block_perception_radius = v;
        applied.insert("block_perception_radius".into(), json!(v));
    }
    if let Some(v) = input.snapshot_interval_ms
        && v != candidate.snapshot_interval_ms
    {
        candidate.snapshot_interval_ms = v;
        applied.insert("snapshot_interval_ms".into(), json!(v));
    }
    if let Some(v) = input.reconnect_initial_delay_ms
        && v != candidate.reconnect_initial_delay_ms
    {
        candidate.reconnect_initial_delay_ms = v;
        applied.insert("reconnect_initial_delay_ms".into(), json!(v));
    }
    if let Some(v) = input.reconnect_max_delay_ms
        && v != candidate.reconnect_max_delay_ms
    {
        candidate.reconnect_max_delay_ms = v;
        applied.insert("reconnect_max_delay_ms".into(), json!(v));
    }
    if let Some(v) = input.command_timeout_secs
        && v != candidate.command_timeout_secs
    {
        candidate.command_timeout_secs = v;
        applied.insert("command_timeout_secs".into(), json!(v));
    }
    if let Some(v) = input.fly_timeout_secs
        && v != candidate.fly_timeout_secs
    {
        candidate.fly_timeout_secs = v;
        applied.insert("fly_timeout_secs".into(), json!(v));
    }
    if let Some(v) = input.mcp_token
        && v != candidate.mcp_token
    {
        candidate.mcp_token = v;
        // The token is a credential — the response only ever shows the
        // redacted form.
        applied.insert("mcp_token".into(), json!("***"));
    }
    if let Some(v) = input.mcp_auth_enabled
        && v != candidate.mcp_auth_enabled
    {
        candidate.mcp_auth_enabled = v;
        // Not a connection field and not a transport-restart field — the
        // auth middleware hot-reads config, so no reconnect is triggered.
        applied.insert("mcp_auth_enabled".into(), json!(v));
    }
    if let Some(v) = input.mcp_transport {
        let transport = parse_transport(&v)?;
        if transport != candidate.mcp_transport {
            candidate.mcp_transport = transport;
            applied.insert("mcp_transport".into(), json!(transport_to_str(transport)));
        }
    }
    let mut new_language: Option<Language> = None;
    if let Some(v) = input.language {
        let language = parse_language(&v)?;
        if language != candidate.language {
            candidate.language = language;
            applied.insert("language".into(), json!(language_to_str(language)));
            new_language = Some(language);
        }
    }

    // Validate BEFORE any global side effect: the old order called
    // i18n::set() inside the loop above, so a REJECTED update had already
    // flipped the UI language globally while leaving the config unchanged.
    candidate.validate().map_err(BotError::InvalidParams)?;

    if let Some(language) = new_language {
        // Next-frame effect in UI mode; harmless in headless mode.
        crate::i18n::set(language);
    }

    // No file persistence: MINECRAFT_MCP_* environment variables are the
    // configuration source, so runtime updates live in memory only.
    // Connection fields changed → the bot must reconnect to honour them.
    let connection_fields_changed = old.mc_address != candidate.mc_address
        || old.mc_port != candidate.mc_port
        || old.ai_username != candidate.ai_username;
    let transport_restart_fields_changed = applied.contains_key("mcp_transport")
        || applied.contains_key("mcp_address")
        || applied.contains_key("mcp_port");

    let mut reconnect_triggered = false;
    if connection_fields_changed {
        if state.is_online() || state.is_connecting() {
            // Online/connecting: the CONNECT LOOP owns the restart flag
            // (single-ownership rule, M-10). request_config_restart +
            // request_disconnect tear the running session down; the connect
            // loop's checkpoint consumes the flag, clears the disconnect
            // request, resets the cancel token and reconnects in-place with
            // the fresh config. The bot thread never exits, so the headless
            // supervisor keeps polling the same handle and must NOT consume
            // the flag — otherwise it could spawn a second bot thread while
            // the old loop reconnects (two azalea sessions → kick loop).
            state.request_config_restart();
            state.request_disconnect();
            reconnect_triggered = true;
        } else {
            // Offline: nobody is connected, so the new config is already
            // applied for the next Connect. The restart flag is set for the
            // HEADLESS supervisor, which consumes it only when no bot thread
            // exists (spawn.rs quiet-wait / next-action) and respawns the
            // bot. In UI mode it is stale by design: `connect()`'s entry
            // cleanup discards it (L-22) so a later explicit Disconnect is
            // never converted into a surprise reconnect.
            state.request_config_restart();
        }
    }

    let mut response = serde_json::Map::new();
    response.insert("applied".into(), json!(applied));
    response.insert("reconnect_triggered".into(), json!(reconnect_triggered));
    if transport_restart_fields_changed {
        response.insert(
            "note".into(),
            json!("mcp_transport/mcp_address/mcp_port changes take effect on process restart"),
        );
    }

    serde_json::to_string(&response)
        .map_err(|e| BotError::Internal(format!("failed to serialize response: {e}")))
}

// ── connect_bot ────────────────────────────────────────────────────────────

/// Handle the `connect_bot` MCP tool.
///
/// Spawns the bot connection on a dedicated OS thread (headless — no UI
/// repaints). No-op responses when the bot is already connected or a
/// connection attempt is already in flight. Works offline (that is the
/// point of the tool).
pub fn connect_bot(
    state: &Arc<SharedState>,
    receiver: &ReceiverSlot,
    sender: &BotCommandSender,
) -> Result<String, BotError> {
    if state.is_online() {
        return Ok("already connected".to_string());
    }
    if state.is_connecting() || !state.try_begin_connecting() {
        return Ok("connection already in progress".to_string());
    }

    match crate::bot::spawn::spawn_bot_connection(
        Arc::clone(state),
        receiver.clone(),
        sender.clone(),
        None,
    ) {
        Ok(()) => Ok("connection started (headless spawn, no UI repaints)".to_string()),
        Err(e) => {
            // The spawn failed — the connecting flag would otherwise stay
            // set forever (no thread exists to run the ClearGuard).
            state.clear_connecting();
            Err(BotError::Internal(format!(
                "failed to spawn bot connection thread: {e}"
            )))
        }
    }
}

// ── disconnect_bot ─────────────────────────────────────────────────────────

/// Handle the `disconnect_bot` MCP tool.
///
/// Requests a disconnect; the connect loop stops retrying and the bot
/// thread exits. Works offline (idempotent no-op in that case).
pub fn disconnect_bot(state: &Arc<SharedState>) -> Result<String, BotError> {
    state.request_disconnect();
    Ok("disconnect requested".to_string())
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::create_command_channel;

    /// Fresh state with a KNOWN token so redaction can be asserted.
    fn state_with_known_token() -> Arc<SharedState> {
        let state = SharedState::new(AppConfig::default());
        state.update_config(|cfg| cfg.mcp_token = "super-secret-token-12345".to_string());
        Arc::new(state)
    }

    // -- get_settings ---------------------------------------------------------

    #[test]
    fn test_get_settings_redacts_token() {
        let state = state_with_known_token();
        let result = get_settings(&state).expect("get_settings should succeed");

        assert!(
            result.contains("\"mcp_token\": \"***\""),
            "token must be redacted, got: {result}"
        );
        assert!(
            !result.contains("super-secret-token-12345"),
            "real token must never appear in get_settings output"
        );
        // Runtime block present.
        assert!(result.contains("\"online\": false"));
        assert!(result.contains("\"connecting\": false"));
        assert!(result.contains("\"mcp_server_status\""));
    }

    #[test]
    fn test_get_settings_reflects_runtime_flags() {
        let state = state_with_known_token();
        state.set_online(true);
        let result = get_settings(&state).expect("get_settings should succeed");
        assert!(result.contains("\"online\": true"));
    }

    #[test]
    fn test_get_settings_includes_auth_enabled() {
        let state = state_with_known_token();
        let result = get_settings(&state).expect("get_settings should succeed");
        assert!(
            result.contains("\"mcp_auth_enabled\": false"),
            "get_settings must expose the auth switch, got: {result}"
        );
    }

    // -- update_settings: validation ------------------------------------------

    #[test]
    fn test_update_settings_invalid_port_rejected() {
        let state = state_with_known_token();
        let input = UpdateSettingsInput {
            mc_port: Some(0),
            ..Default::default()
        };
        let result = update_settings(&state, input);
        assert!(
            matches!(&result, Err(BotError::InvalidParams(msg)) if msg.contains("mc_port")),
            "got: {result:?}"
        );
        // In-memory config untouched.
        assert_eq!(state.read_config().mc_port, AppConfig::default().mc_port);
    }

    #[test]
    fn test_update_settings_unknown_transport_rejected() {
        let state = state_with_known_token();
        let input = UpdateSettingsInput {
            mcp_transport: Some("carrier-pigeon".into()),
            ..Default::default()
        };
        let result = update_settings(&state, input);
        assert!(
            matches!(&result, Err(BotError::InvalidParams(msg)) if msg.contains("mcp_transport")),
            "got: {result:?}"
        );
        assert_eq!(
            state.read_config().mcp_transport,
            AppConfig::default().mcp_transport
        );
    }

    #[test]
    fn test_update_settings_unknown_language_rejected() {
        let state = state_with_known_token();
        let input = UpdateSettingsInput {
            language: Some("klingon".into()),
            ..Default::default()
        };
        let result = update_settings(&state, input);
        assert!(
            matches!(&result, Err(BotError::InvalidParams(msg)) if msg.contains("language")),
            "got: {result:?}"
        );
    }

    // -- update_settings: partial update + persistence ------------------------

    #[test]
    fn test_update_settings_partial_update_preserves_other_fields() {
        let state = state_with_known_token();
        let input = UpdateSettingsInput {
            task_name: Some("diamond-mining".into()),
            ..Default::default()
        };

        let result = update_settings(&state, input).expect("update should succeed");

        let config = state.read_config().clone();
        assert_eq!(config.task_name, "diamond-mining");
        // All other fields keep their defaults.
        assert_eq!(config.mc_address, AppConfig::default().mc_address);
        assert_eq!(config.mc_port, AppConfig::default().mc_port);
        assert_eq!(config.mcp_token, "super-secret-token-12345");

        // Response shape: only the changed field in `applied`.
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(value["reconnect_triggered"], json!(false));
        let applied = value["applied"].as_object().unwrap();
        assert_eq!(applied.len(), 1);
        assert_eq!(applied["task_name"], json!("diamond-mining"));
        assert!(value.get("note").is_none());

        // The token is NOT re-exposed through `applied`.
        assert!(!result.contains("super-secret-token-12345"));

        // The update is memory-only: no file is ever written (the env-var
        // config source has no file I/O by construction).
    }

    #[test]
    fn test_update_settings_same_value_not_in_applied() {
        let state = state_with_known_token();
        let input = UpdateSettingsInput {
            mc_port: Some(AppConfig::default().mc_port),
            ..Default::default()
        };
        let result = update_settings(&state, input).expect("update should succeed");
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert!(
            value["applied"].as_object().unwrap().is_empty(),
            "unchanged values must not appear in applied: {result}"
        );
    }

    #[test]
    fn test_update_settings_auth_enabled_toggles() {
        let state = state_with_known_token();
        // Force a known starting value so the toggle is a real change.
        state.update_config(|cfg| cfg.mcp_auth_enabled = false);
        let input = UpdateSettingsInput {
            mcp_auth_enabled: Some(true),
            ..Default::default()
        };

        let result = update_settings(&state, input).expect("update should succeed");

        // In-memory value applied.
        assert!(state.read_config().mcp_auth_enabled);

        // Response reports the change.
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(value["applied"]["mcp_auth_enabled"], json!(true));
        assert_eq!(value["reconnect_triggered"], json!(false));
        assert!(
            value.get("note").is_none(),
            "auth toggle must not carry a restart note: {result}"
        );
    }

    #[test]
    fn test_update_settings_auth_enabled_same_value_not_applied() {
        let state = state_with_known_token();
        // Default is false — setting false again must be a no-op.
        state.update_config(|cfg| cfg.mcp_auth_enabled = false);
        let input = UpdateSettingsInput {
            mcp_auth_enabled: Some(false),
            ..Default::default()
        };

        let result = update_settings(&state, input).expect("update should succeed");
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert!(
            value["applied"].as_object().unwrap().is_empty(),
            "unchanged auth value must not appear in applied: {result}"
        );
    }

    // -- update_settings: reconnect semantics ---------------------------------

    #[test]
    fn test_update_settings_connection_change_while_online_sets_restart() {
        let state = state_with_known_token();
        state.set_online(true);
        let input = UpdateSettingsInput {
            mc_address: Some("play.example.com".into()),
            ..Default::default()
        };

        let result = update_settings(&state, input).expect("update should succeed");
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(value["reconnect_triggered"], json!(true));

        // Restart flag consumed by the connect loop; disconnect requested.
        assert!(state.take_config_restart());
        assert!(!state.take_config_restart(), "flag must be single-shot");
        assert!(state.is_disconnect_requested());
        assert_eq!(state.read_config().mc_address, "play.example.com");
    }

    #[test]
    fn test_update_settings_connection_change_while_offline_no_disconnect() {
        let state = state_with_known_token();
        let input = UpdateSettingsInput {
            mc_port: Some(25566),
            ..Default::default()
        };

        let result = update_settings(&state, input).expect("update should succeed");
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(
            value["reconnect_triggered"],
            json!(false),
            "offline → no live reconnect"
        );

        // Restart flag is still set for the headless supervisor, but no
        // disconnect is requested.
        assert!(state.take_config_restart());
        assert!(!state.is_disconnect_requested());
    }

    #[test]
    fn test_update_settings_transport_change_includes_note() {
        let state = state_with_known_token();
        // Default transport is Http → switching to stdio is a change.
        let input = UpdateSettingsInput {
            mcp_transport: Some("stdio".into()),
            ..Default::default()
        };

        let result = update_settings(&state, input).expect("update should succeed");
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(value["applied"]["mcp_transport"], json!("stdio"));
        assert!(
            value["note"]
                .as_str()
                .is_some_and(|n| n.contains("process restart")),
            "transport change must carry the restart note: {result}"
        );
        assert_eq!(state.read_config().mcp_transport, McpTransport::Stdio);
    }

    #[test]
    fn test_update_settings_language_change_applies_and_sets_i18n() {
        let state = state_with_known_token();
        // Force a known starting language — the default follows the system
        // locale (which may already be zh_cn on this machine).
        state.update_config(|cfg| cfg.language = Language::En);
        let input = UpdateSettingsInput {
            language: Some("zh_cn".into()),
            ..Default::default()
        };

        let result = update_settings(&state, input).expect("update should succeed");
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(value["applied"]["language"], json!("zh_cn"));
        assert_eq!(state.read_config().language, Language::ZhCn);
        assert_eq!(crate::i18n::current(), Language::ZhCn);

        // Restore the global i18n state for other tests.
        crate::i18n::set(Language::En);
    }

    // -- connect_bot / disconnect_bot ------------------------------------------

    #[test]
    fn test_connect_bot_when_online_returns_already_connected() {
        let state = state_with_known_token();
        state.set_online(true);
        let (sender, receiver) = create_command_channel(4, Arc::clone(&state));
        let slot: ReceiverSlot = Arc::new(std::sync::Mutex::new(Some(receiver)));

        let result = connect_bot(&state, &slot, &sender).expect("connect_bot should succeed");
        assert_eq!(result, "already connected");
        // No spawn happened: no connecting flag, no stored thread handle.
        assert!(!state.is_connecting());
        assert!(state.take_bot_thread_handle().is_none());
    }

    #[test]
    fn test_connect_bot_when_connecting_returns_in_progress() {
        let state = state_with_known_token();
        assert!(state.try_begin_connecting());
        let (sender, receiver) = create_command_channel(4, Arc::clone(&state));
        let slot: ReceiverSlot = Arc::new(std::sync::Mutex::new(Some(receiver)));

        let result = connect_bot(&state, &slot, &sender).expect("connect_bot should succeed");
        assert_eq!(result, "connection already in progress");
        // Flag untouched — the in-flight attempt still owns it.
        assert!(state.is_connecting());
        state.clear_connecting();
    }

    #[test]
    fn test_disconnect_bot_requests_disconnect() {
        let state = state_with_known_token();
        let result = disconnect_bot(&state).expect("disconnect_bot should succeed");
        assert_eq!(result, "disconnect requested");
        assert!(state.is_disconnect_requested());
    }
}
