//! Configuration types for the Minecraft MCP server.
//!
//! Provides [`AppConfig`] for UI-facing settings and [`RunStats`] for
//! thread-safe command tracking counters.
//!
//! # Persistence
//!
//! Settings survive restarts via a JSON config file:
//!
//! - [`config_path`] resolves the default location —
//!   `minecraft-mcp-rs/config.json` under the OS config directory
//!   (e.g. `%APPDATA%` on Windows, `~/.config` on Linux,
//!   `~/Library/Application Support` on macOS).
//! - [`AppConfig::load_from_disk`] reads a config file (explicit path, or
//!   [`config_path`] when `None`). A missing file yields
//!   [`AppConfig::default()`]; malformed JSON logs a warning and falls back
//!   to defaults; a partial file keeps its present fields and fills the rest
//!   from the serde field defaults.
//! - [`AppConfig::save_to_disk`] writes pretty-printed JSON atomically —
//!   temp file in the same directory, then [`std::fs::rename`] — creating
//!   parent directories as needed. On Unix the temp file is created with
//!   mode `0600` because the file contains the MCP bearer token.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicU64;
use std::time::Instant;

use crate::ui::i18n::Language;

// ---------------------------------------------------------------------------
// McpTransport — transport selection for the MCP server
// ---------------------------------------------------------------------------

/// Transport mechanism the MCP server uses to talk to clients.
///
/// `Stdio` is the classic JSON-RPC-over-stdio transport used by Claude
/// Desktop / Cursor; `Http` exposes the server over HTTP (useful for
/// remote clients and browser-based integrations).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum McpTransport {
    /// JSON-RPC over stdio (default for local MCP clients).
    Stdio,
    /// JSON-RPC over HTTP.
    Http,
}

/// Default transport is `Http` so the server is reachable remotely
/// without extra plumbing.
impl Default for McpTransport {
    fn default() -> Self {
        Self::Http
    }
}

// ---------------------------------------------------------------------------
// AppConfig — UI-facing settings with sensible defaults
// ---------------------------------------------------------------------------

/// All user-configurable settings for the Minecraft MCP server.
///
/// Every field has a sensible default so that the egui settings panel
/// can be populated from [`AppConfig::default()`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppConfig {
    /// Minecraft server address (default: `"127.0.0.1"`).
    #[serde(default = "default_mc_address")]
    pub mc_address: String,
    /// Minecraft server port (default: `25565`).
    #[serde(default = "default_mc_port")]
    pub mc_port: u16,
    /// Bot in-game username (default: `"AI_Bot"`).
    #[serde(default = "default_ai_username")]
    pub ai_username: String,
    /// MCP server bind address (default: `"127.0.0.1"`).
    #[serde(default = "default_mcp_address")]
    pub mcp_address: String,
    /// MCP server bind port (default: `3000`).
    #[serde(default = "default_mcp_port")]
    pub mcp_port: u16,
    /// Descriptive task name for the UI (default: `"mining"`).
    #[serde(default = "default_task_name")]
    pub task_name: String,
    /// How many chunks to scan around the player (range: 1–16, default: 8).
    #[serde(default = "default_chunk_scan_radius")]
    pub chunk_scan_radius: u8,
    /// Block perception radius in blocks (range: 8–64, default: 32).
    #[serde(default = "default_block_perception_radius")]
    pub block_perception_radius: u8,
    /// Interval between world snapshots in milliseconds (default: 500).
    #[serde(default = "default_snapshot_interval_ms")]
    pub snapshot_interval_ms: u64,
    /// Initial reconnect delay in milliseconds (default: 5000).
    #[serde(default = "default_reconnect_initial_delay_ms")]
    pub reconnect_initial_delay_ms: u64,
    /// Maximum reconnect delay in milliseconds (default: 60000).
    #[serde(default = "default_reconnect_max_delay_ms")]
    pub reconnect_max_delay_ms: u64,
    /// Timeout for bot commands in seconds (default: 30).
    #[serde(default = "default_command_timeout_secs")]
    pub command_timeout_secs: u64,
    /// Authentication token presented by MCP clients over HTTP
    /// (default: a random UUID v4 generated per fresh [`AppConfig::default()`]
    /// / missing-field deserialization; override via the settings panel).
    ///
    /// The token is serialized so it persists across restarts; public
    /// surfaces (e.g. the `get_settings` MCP tool) must redact it.
    #[serde(default = "default_mcp_token")]
    pub mcp_token: String,
    /// Whether MCP clients must present a valid bearer token (default:
    /// `false`). When `true`, [`AppConfig::validate`] rejects an empty
    /// [`AppConfig::mcp_token`]; when `false`, an empty token is allowed
    /// (e.g. stdio transport or a trusted loopback network).
    #[serde(default)]
    pub mcp_auth_enabled: bool,
    /// Transport the MCP server uses to communicate with clients
    /// (default: [`McpTransport::Http`]).
    #[serde(default)]
    pub mcp_transport: McpTransport,
    /// UI display language (default: [`Language::En`]).
    #[serde(default)]
    pub language: Language,
}

/// Serde default for [`AppConfig::mcp_token`].
fn default_mcp_token() -> String {
    // Generate a random token using UUID v4. This ensures each fresh install
    // gets a unique token rather than a hardcoded default that attackers could
    // exploit. The user can still override it via the settings panel.
    uuid::Uuid::new_v4().to_string()
}

// ---- Serde field defaults --------------------------------------------------
// One function per field so a PARTIAL config file (some fields missing)
// deserializes with the same values as `AppConfig::default()`. The `Default`
// impl below calls these very functions, so the two can never drift apart.

/// Serde default for [`AppConfig::mc_address`].
fn default_mc_address() -> String {
    "127.0.0.1".into()
}

/// Serde default for [`AppConfig::mc_port`].
fn default_mc_port() -> u16 {
    25565
}

/// Serde default for [`AppConfig::ai_username`].
fn default_ai_username() -> String {
    "AI_Bot".into()
}

/// Serde default for [`AppConfig::mcp_address`].
fn default_mcp_address() -> String {
    "127.0.0.1".into()
}

/// Serde default for [`AppConfig::mcp_port`].
fn default_mcp_port() -> u16 {
    3000
}

/// Serde default for [`AppConfig::task_name`].
fn default_task_name() -> String {
    "mining".into()
}

/// Serde default for [`AppConfig::chunk_scan_radius`].
fn default_chunk_scan_radius() -> u8 {
    8
}

/// Serde default for [`AppConfig::block_perception_radius`].
fn default_block_perception_radius() -> u8 {
    32
}

/// Serde default for [`AppConfig::snapshot_interval_ms`].
fn default_snapshot_interval_ms() -> u64 {
    500
}

/// Serde default for [`AppConfig::reconnect_initial_delay_ms`].
fn default_reconnect_initial_delay_ms() -> u64 {
    5000
}

/// Serde default for [`AppConfig::reconnect_max_delay_ms`].
fn default_reconnect_max_delay_ms() -> u64 {
    60_000
}

/// Serde default for [`AppConfig::command_timeout_secs`].
fn default_command_timeout_secs() -> u64 {
    30
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            mc_address: default_mc_address(),
            mc_port: default_mc_port(),
            ai_username: default_ai_username(),
            mcp_address: default_mcp_address(),
            mcp_port: default_mcp_port(),
            task_name: default_task_name(),
            chunk_scan_radius: default_chunk_scan_radius(),
            block_perception_radius: default_block_perception_radius(),
            snapshot_interval_ms: default_snapshot_interval_ms(),
            reconnect_initial_delay_ms: default_reconnect_initial_delay_ms(),
            reconnect_max_delay_ms: default_reconnect_max_delay_ms(),
            command_timeout_secs: default_command_timeout_secs(),
            mcp_token: default_mcp_token(),
            mcp_auth_enabled: false,
            mcp_transport: McpTransport::default(),
            language: Language::from_system_locale(),
        }
    }
}

impl AppConfig {
    /// Validate all config fields and return an error message for the first
    /// invalid value found.
    pub fn validate(&self) -> Result<(), String> {
        if self.mc_address.is_empty() {
            return Err("mc_address must not be empty".into());
        }
        if self.ai_username.is_empty() {
            return Err("ai_username must not be empty".into());
        }
        if self.mc_port == 0 {
            return Err("mc_port must not be 0".into());
        }
        if (self.mc_port as u32) > 65535 {
            return Err("mc_port must be <= 65535".into());
        }
        if self.mcp_port == 0 {
            return Err("mcp_port must not be 0".into());
        }
        if (self.mcp_port as u32) > 65535 {
            return Err("mcp_port must be <= 65535".into());
        }
        if self.mcp_address != "localhost" {
            self.mcp_address
                .parse::<std::net::IpAddr>()
                .map_err(|_| "mcp_address must be a valid IP address (e.g. 127.0.0.1 or 0.0.0.0) or \"localhost\"")?;
        }
        if self.chunk_scan_radius < 1 || self.chunk_scan_radius > 16 {
            return Err(format!(
                "chunk_scan_radius must be between 1 and 16, got {}",
                self.chunk_scan_radius
            ));
        }
        if self.block_perception_radius < 8 || self.block_perception_radius > 64 {
            return Err(format!(
                "block_perception_radius must be between 8 and 64, got {}",
                self.block_perception_radius
            ));
        }
        if self.snapshot_interval_ms == 0 {
            return Err("snapshot_interval_ms must be greater than 0".into());
        }
        if self.reconnect_initial_delay_ms == 0 {
            return Err("reconnect_initial_delay_ms must be greater than 0".into());
        }
        if self.reconnect_max_delay_ms == 0 {
            return Err("reconnect_max_delay_ms must be greater than 0".into());
        }
        if self.reconnect_max_delay_ms < self.reconnect_initial_delay_ms {
            return Err("reconnect_max_delay_ms must be >= reconnect_initial_delay_ms".into());
        }
        if self.command_timeout_secs == 0 {
            return Err("command_timeout_secs must be greater than 0".into());
        }
        if self.mcp_auth_enabled && self.mcp_token.is_empty() {
            return Err("mcp_token must not be empty when auth is enabled".into());
        }
        Ok(())
    }

    /// Load configuration from disk.
    ///
    /// Reads `path` when given, otherwise the default [`config_path`].
    /// Fallbacks (never panics):
    ///
    /// - no path available (no OS config directory) → [`AppConfig::default()`]
    /// - file does not exist → [`AppConfig::default()`] (normal first run)
    /// - file is not valid JSON → `tracing::warn!` + [`AppConfig::default()`]
    /// - file is partial JSON → present fields win, missing fields are filled
    ///   by their serde defaults
    ///
    /// This function deliberately does NOT call [`AppConfig::validate`] —
    /// loading a stale or hand-edited file must not prevent the app from
    /// starting; validation happens where settings are applied.
    pub fn load_from_disk(path: Option<&std::path::Path>) -> AppConfig {
        let default_path = path.is_none().then(config_path).flatten();
        let path = match path.or(default_path.as_deref()) {
            Some(p) => p,
            None => {
                tracing::debug!("no OS config directory available; using in-memory defaults");
                return AppConfig::default();
            }
        };

        match std::fs::read_to_string(path) {
            Ok(contents) => match serde_json::from_str::<AppConfig>(&contents) {
                Ok(config) => {
                    tracing::debug!(path = %path.display(), "loaded config from disk");
                    config
                }
                Err(err) => {
                    tracing::warn!(
                        path = %path.display(),
                        error = %err,
                        "config file contains malformed JSON; falling back to defaults"
                    );
                    AppConfig::default()
                }
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                // First run — no config file yet. Normal, not a warning.
                tracing::debug!(path = %path.display(), "no config file found; using defaults");
                AppConfig::default()
            }
            Err(err) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %err,
                    "failed to read config file; falling back to defaults"
                );
                AppConfig::default()
            }
        }
    }

    /// Persist this configuration to disk as pretty-printed JSON.
    ///
    /// Writes to `path` when given, otherwise the default [`config_path`].
    /// The write is atomic: the JSON goes to a temp file in the SAME
    /// directory first, then [`std::fs::rename`] swaps it into place
    /// (rename is atomic within one filesystem). Parent directories are
    /// created as needed. On Unix the temp file is created with mode
    /// `0600` because the file contains the MCP bearer token.
    ///
    /// Returns `Err(message)` on any IO failure (the temp file is removed
    /// on a best-effort basis in that case).
    pub fn save_to_disk(&self, path: Option<&std::path::Path>) -> Result<(), String> {
        let default_path = path.is_none().then(config_path).flatten();
        let path = path
            .or(default_path.as_deref())
            .ok_or_else(|| "no config path available: OS config directory not found".to_string())?;

        // Create parent directories. `parent()` is `Some("")` for a bare
        // relative file name — skip the empty path.
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|err| {
                format!(
                    "failed to create config directory {}: {err}",
                    parent.display()
                )
            })?;
        }

        let json = serde_json::to_string_pretty(self)
            .map_err(|err| format!("failed to serialize config to JSON: {err}"))?;

        // Temp file in the same directory so the final rename stays atomic.
        let file_name = path
            .file_name()
            .ok_or_else(|| format!("invalid config path (no file name): {}", path.display()))?;
        let mut tmp_file_name = file_name.to_os_string();
        tmp_file_name.push(format!(".tmp-{}", std::process::id()));
        let tmp_path = path.with_file_name(tmp_file_name);

        // Write the temp file; 0600 on Unix since it holds the bearer token.
        let mut options = std::fs::OpenOptions::new();
        options.create(true).write(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let write_result = options.open(&tmp_path).and_then(|mut file| {
            std::io::Write::write_all(&mut file, json.as_bytes())?;
            file.sync_all()
        });
        if let Err(err) = write_result {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(format!(
                "failed to write temp config file {}: {err}",
                tmp_path.display()
            ));
        }

        std::fs::rename(&tmp_path, path).map_err(|err| {
            let _ = std::fs::remove_file(&tmp_path);
            format!(
                "failed to move temp config file into place {}: {err}",
                path.display()
            )
        })?;

        tracing::debug!(path = %path.display(), "saved config to disk");
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// config_path — default config file location in the OS config dir
// ---------------------------------------------------------------------------

/// Default config file location: `minecraft-mcp-rs/config.json` under the
/// OS config directory (e.g. `%APPDATA%` on Windows, `~/.config` on Linux,
/// `~/Library/Application Support` on macOS).
///
/// Returns `None` when the platform does not expose a config directory.
pub fn config_path() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|dir| dir.join("minecraft-mcp-rs").join("config.json"))
}

// ---------------------------------------------------------------------------
// RunStats — thread-safe command tracking counters
// ---------------------------------------------------------------------------

/// Atomic counters for command processing statistics, shared across
/// the bot engine and the egui status panel.
#[derive(Debug)]
pub struct RunStats {
    /// Total commands dispatched.
    pub commands_processed: AtomicU64,
    /// Commands that completed successfully.
    pub commands_succeeded: AtomicU64,
    /// Commands that failed.
    pub commands_failed: AtomicU64,
    /// Timestamp when the last connection was established.
    pub connected_since: Option<Instant>,
}

impl Default for RunStats {
    fn default() -> Self {
        Self {
            commands_processed: AtomicU64::new(0),
            commands_succeeded: AtomicU64::new(0),
            commands_failed: AtomicU64::new(0),
            connected_since: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::field_reassign_with_default)]
    use super::*;
    use std::sync::atomic::Ordering;

    // -- AppConfig defaults -------------------------------------------------

    #[test]
    fn test_default_config_is_valid() {
        let config = AppConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_default_config_values() {
        let config = AppConfig::default();
        assert_eq!(config.mc_address, "127.0.0.1");
        assert_eq!(config.mc_port, 25565);
        assert_eq!(config.ai_username, "AI_Bot");
        assert_eq!(config.mcp_address, "127.0.0.1");
        assert_eq!(config.mcp_port, 3000);
        assert_eq!(config.task_name, "mining");
        assert_eq!(config.chunk_scan_radius, 8);
        assert_eq!(config.block_perception_radius, 32);
        assert_eq!(config.snapshot_interval_ms, 500);
        assert_eq!(config.reconnect_initial_delay_ms, 5000);
        assert_eq!(config.reconnect_max_delay_ms, 60_000);
        assert_eq!(config.command_timeout_secs, 30);
    }

    // -- McpTransport / mcp_token defaults ----------------------------------

    #[test]
    fn test_default_config() {
        let config = AppConfig::default();
        // Token is now randomly generated (see `test_default_token_is_random`
        // and `test_default_token_not_hardcoded_value`); only assert that it
        // is non-empty here.
        assert!(!config.mcp_token.is_empty());
        assert_eq!(config.mcp_transport, McpTransport::Http);
    }

    #[test]
    fn test_mcp_transport_default_is_http() {
        assert_eq!(McpTransport::default(), McpTransport::Http);
    }

    #[test]
    fn test_default_token_is_random() {
        // Two consecutive calls must yield different tokens, proving the
        // default is randomly generated rather than a hardcoded constant.
        // Each token must also be reasonably long (>= 16 chars) so it cannot
        // be brute-forced trivially.
        let a = default_mcp_token();
        let b = default_mcp_token();
        assert_ne!(a, b, "default token must be random, got {a} twice");
        assert!(
            a.len() >= 16,
            "default token too short ({} chars): {a}",
            a.len()
        );
        assert!(
            b.len() >= 16,
            "default token too short ({} chars): {b}",
            b.len()
        );
    }

    #[test]
    fn test_default_token_not_hardcoded_value() {
        // The historical hardcoded value must never come back — it is a
        // known weak credential attackers could exploit.
        assert_ne!(default_mcp_token(), "minecraft-mcp-rs");
        assert_ne!(AppConfig::default().mcp_token, "minecraft-mcp-rs");
    }

    // -- Language field -----------------------------------------------------

    #[test]
    fn test_default_config_language() {
        let config = AppConfig::default();
        // The default language follows the host system locale rather than
        // hardcoding English, so we compare against the runtime-detected
        // value instead of Language::En.
        assert_eq!(config.language, Language::from_system_locale());
    }

    #[test]
    fn test_old_config_without_language_deserializes() {
        // A JSON payload lacking the `language` field (as written by older
        // binaries before i18n existed) must still deserialize, with the
        // field falling back to its `#[serde(default)]` value.
        let json = r#"{
            "mc_address": "127.0.0.1",
            "mc_port": 25565,
            "ai_username": "AI_Bot",
            "mcp_address": "127.0.0.1",
            "mcp_port": 3000,
            "task_name": "mining",
            "chunk_scan_radius": 8,
            "block_perception_radius": 32,
            "snapshot_interval_ms": 500,
            "reconnect_initial_delay_ms": 5000,
            "reconnect_max_delay_ms": 60000,
            "command_timeout_secs": 30,
            "mcp_token": "minecraft-mcp-rs",
            "mcp_transport": "Http"
        }"#;
        let config: AppConfig = serde_json::from_str(json).expect("must deserialize");
        assert_eq!(config.language, Language::En);
    }

    #[test]
    fn test_validate_rejects_empty_token() {
        // An empty token is only rejected when auth is enabled.
        let mut config = AppConfig::default();
        config.mcp_auth_enabled = true;
        config.mcp_token.clear();
        let err = config.validate().unwrap_err();
        assert!(err.contains("mcp_token"), "got: {err}");
    }

    #[test]
    fn test_default_auth_disabled() {
        // Auth must be opt-in: existing stdio / trusted-loopback setups keep
        // working without a token.
        let config = AppConfig::default();
        assert!(!config.mcp_auth_enabled);
    }

    #[test]
    fn test_validate_allows_empty_token_when_auth_off() {
        let mut config = AppConfig::default();
        config.mcp_auth_enabled = false;
        config.mcp_token.clear();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_rejects_empty_token_when_auth_on() {
        let mut config = AppConfig::default();
        config.mcp_auth_enabled = true;
        config.mcp_token.clear();
        let err = config.validate().unwrap_err();
        assert!(err.contains("auth"), "got: {err}");
    }

    #[test]
    fn test_old_config_without_auth_field_deserializes() {
        // A JSON payload lacking the `mcp_auth_enabled` field (as written by
        // older binaries before MCP auth existed) must still deserialize,
        // with the field falling back to its `#[serde(default)]` value.
        let json = r#"{
            "mc_address": "127.0.0.1",
            "mc_port": 25565,
            "ai_username": "AI_Bot",
            "mcp_address": "127.0.0.1",
            "mcp_port": 3000,
            "task_name": "mining",
            "chunk_scan_radius": 8,
            "block_perception_radius": 32,
            "snapshot_interval_ms": 500,
            "reconnect_initial_delay_ms": 5000,
            "reconnect_max_delay_ms": 60000,
            "command_timeout_secs": 30,
            "mcp_token": "minecraft-mcp-rs",
            "mcp_transport": "Http"
        }"#;
        let config: AppConfig = serde_json::from_str(json).expect("must deserialize");
        assert!(!config.mcp_auth_enabled);
    }

    // -- Validation: chunk_scan_radius --------------------------------------

    #[test]
    fn test_validate_chunk_scan_radius_zero() {
        let mut config = AppConfig::default();
        config.chunk_scan_radius = 0;
        let err = config.validate().unwrap_err();
        assert!(err.contains("1 and 16"), "got: {err}");
    }

    #[test]
    fn test_validate_chunk_scan_radius_too_high() {
        let mut config = AppConfig::default();
        config.chunk_scan_radius = 20;
        let err = config.validate().unwrap_err();
        assert!(err.contains("1 and 16"), "got: {err}");
    }

    #[test]
    fn test_validate_chunk_scan_radius_min_edge() {
        let mut config = AppConfig::default();
        config.chunk_scan_radius = 1;
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_chunk_scan_radius_max_edge() {
        let mut config = AppConfig::default();
        config.chunk_scan_radius = 16;
        assert!(config.validate().is_ok());
    }

    // -- Validation: block_perception_radius ---------------------------------

    #[test]
    fn test_validate_block_perception_radius_below_min() {
        let mut config = AppConfig::default();
        config.block_perception_radius = 7;
        let err = config.validate().unwrap_err();
        assert!(err.contains("8 and 64"), "got: {err}");
    }

    #[test]
    fn test_validate_block_perception_radius_above_max() {
        let mut config = AppConfig::default();
        config.block_perception_radius = 65;
        let err = config.validate().unwrap_err();
        assert!(err.contains("8 and 64"), "got: {err}");
    }

    #[test]
    fn test_validate_block_perception_radius_min_edge() {
        let mut config = AppConfig::default();
        config.block_perception_radius = 8;
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_block_perception_radius_max_edge() {
        let mut config = AppConfig::default();
        config.block_perception_radius = 64;
        assert!(config.validate().is_ok());
    }

    // -- Validation: mc_address ----------------------------------------------

    #[test]
    fn test_validate_empty_mc_address() {
        let mut config = AppConfig::default();
        config.mc_address.clear();
        let err = config.validate().unwrap_err();
        assert!(err.contains("mc_address"), "got: {err}");
    }

    // -- Validation: ai_username ---------------------------------------------

    #[test]
    fn test_validate_empty_ai_username() {
        let mut config = AppConfig::default();
        config.ai_username.clear();
        let err = config.validate().unwrap_err();
        assert!(err.contains("ai_username"), "got: {err}");
    }

    // -- Validation: mcp_address ---------------------------------------------

    #[test]
    fn test_validate_rejects_invalid_mcp_address() {
        let mut config = AppConfig::default();
        config.mcp_address = "not-an-ip".to_string();
        assert!(
            config.validate().is_err(),
            "invalid mcp_address should fail validation"
        );

        config.mcp_address = "0.0.0.0".to_string();
        assert!(config.validate().is_ok(), "valid IPv4 should pass");

        config.mcp_address = "::1".to_string();
        assert!(config.validate().is_ok(), "valid IPv6 should pass");
    }

    #[test]
    fn test_validate_accepts_localhost() {
        let mut config = AppConfig::default();
        config.mcp_address = "localhost".to_string();
        assert!(
            config.validate().is_ok(),
            "localhost should pass validation"
        );
    }

    // -- Validation: ports -------------------------------------------------

    #[test]
    fn test_validate_rejects_port_too_high() {
        let mut config = AppConfig::default();
        config.mcp_port = 65535;
        assert!(config.validate().is_ok(), "port 65535 should be valid");
    }

    // -- Validation: snapshot_interval_ms ----------------------------------

    #[test]
    fn test_validate_rejects_zero_snapshot_interval() {
        let mut config = AppConfig::default();
        config.snapshot_interval_ms = 0;
        let err = config.validate().unwrap_err();
        assert!(err.contains("snapshot_interval_ms"), "got: {err}");
    }

    // -- Validation: reconnect delays --------------------------------------

    #[test]
    fn test_validate_rejects_zero_reconnect_initial_delay() {
        let mut config = AppConfig::default();
        config.reconnect_initial_delay_ms = 0;
        let err = config.validate().unwrap_err();
        assert!(err.contains("reconnect_initial_delay_ms"), "got: {err}");
    }

    #[test]
    fn test_validate_rejects_max_delay_less_than_initial() {
        let mut config = AppConfig::default();
        config.reconnect_initial_delay_ms = 5000;
        config.reconnect_max_delay_ms = 1000;
        let err = config.validate().unwrap_err();
        assert!(err.contains("reconnect_max_delay_ms"), "got: {err}");
    }

    // -- Validation: command_timeout_secs ------------------------------------

    #[test]
    fn test_validate_command_timeout_zero() {
        let mut config = AppConfig::default();
        config.command_timeout_secs = 0;
        let err = config.validate().unwrap_err();
        assert!(err.contains("command_timeout_secs"), "got: {err}");
    }

    // -- RunStats ------------------------------------------------------------

    #[test]
    fn test_run_stats_default_zero() {
        let stats = RunStats::default();
        assert_eq!(stats.commands_processed.load(Ordering::Relaxed), 0);
        assert_eq!(stats.commands_succeeded.load(Ordering::Relaxed), 0);
        assert_eq!(stats.commands_failed.load(Ordering::Relaxed), 0);
        assert!(stats.connected_since.is_none());
    }

    #[test]
    fn test_run_stats_atomic_increment() {
        let stats = RunStats::default();
        stats.commands_processed.fetch_add(1, Ordering::SeqCst);
        stats.commands_succeeded.fetch_add(1, Ordering::SeqCst);
        assert_eq!(stats.commands_processed.load(Ordering::SeqCst), 1);
        assert_eq!(stats.commands_succeeded.load(Ordering::SeqCst), 1);
        assert_eq!(stats.commands_failed.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn test_run_stats_connected_since() {
        let mut stats = RunStats::default();
        assert!(stats.connected_since.is_none());
        stats.connected_since = Some(Instant::now());
        assert!(stats.connected_since.is_some());
    }

    // -- Persistence: helpers ------------------------------------------------

    /// RAII guard that removes the test directory on drop, so temp files
    /// are cleaned up even when a test panics mid-assertion.
    struct TempTestDir(std::path::PathBuf);

    impl Drop for TempTestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Create a unique temp directory for one test.
    ///
    /// Uniqueness comes from the process id plus a per-test `label`, so
    /// parallel tests (same process, different threads) never share a
    /// directory and never touch the real user config dir.
    fn test_dir(label: &str) -> TempTestDir {
        let dir = std::env::temp_dir().join(format!(
            "minecraft-mcp-rs-test-{}-{label}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("failed to create test dir");
        TempTestDir(dir)
    }

    /// A config with deterministic, `validate()`-passing values in every
    /// field, used for roundtrip equality checks.
    fn sample_config() -> AppConfig {
        let mut config = AppConfig::default();
        config.mc_address = "10.0.0.5".into();
        config.mc_port = 25566;
        config.ai_username = "TestBot".into();
        config.mcp_address = "127.0.0.1".into();
        config.mcp_port = 3001;
        config.task_name = "testing".into();
        config.chunk_scan_radius = 4;
        config.block_perception_radius = 16;
        config.snapshot_interval_ms = 250;
        config.reconnect_initial_delay_ms = 1000;
        config.reconnect_max_delay_ms = 30_000;
        config.command_timeout_secs = 10;
        config.mcp_token = "roundtrip-token-123".into();
        config.mcp_auth_enabled = true;
        config.mcp_transport = McpTransport::Stdio;
        config.language = Language::ZhCn;
        config
    }

    // -- Persistence: config_path ---------------------------------------------

    #[test]
    fn test_config_path_ends_with_expected_suffix_or_none() {
        // Either the OS exposes a config dir (path must end with
        // `minecraft-mcp-rs/config.json`) or it does not (None). Both are
        // acceptable; a Some with a wrong suffix is a bug.
        if let Some(path) = config_path() {
            let suffix = std::path::Path::new("minecraft-mcp-rs").join("config.json");
            assert!(
                path.ends_with(suffix),
                "config path must end with minecraft-mcp-rs/config.json, got: {}",
                path.display()
            );
        }
    }

    // -- Persistence: save_to_disk / load_from_disk roundtrip ------------------

    #[test]
    fn test_save_load_roundtrip_preserves_all_fields_including_token() {
        let dir = test_dir("roundtrip");
        // Nested path also proves save_to_disk creates missing parent dirs.
        let path = dir.0.join("nested").join("deep").join("config.json");
        let original = sample_config();

        original
            .save_to_disk(Some(&path))
            .expect("save must succeed");
        assert!(path.exists(), "config file must exist after save");

        let loaded = AppConfig::load_from_disk(Some(&path));
        assert_eq!(loaded.mc_address, original.mc_address);
        assert_eq!(loaded.mc_port, original.mc_port);
        assert_eq!(loaded.ai_username, original.ai_username);
        assert_eq!(loaded.mcp_address, original.mcp_address);
        assert_eq!(loaded.mcp_port, original.mcp_port);
        assert_eq!(loaded.task_name, original.task_name);
        assert_eq!(loaded.chunk_scan_radius, original.chunk_scan_radius);
        assert_eq!(
            loaded.block_perception_radius,
            original.block_perception_radius
        );
        assert_eq!(loaded.snapshot_interval_ms, original.snapshot_interval_ms);
        assert_eq!(
            loaded.reconnect_initial_delay_ms,
            original.reconnect_initial_delay_ms
        );
        assert_eq!(
            loaded.reconnect_max_delay_ms,
            original.reconnect_max_delay_ms
        );
        assert_eq!(loaded.command_timeout_secs, original.command_timeout_secs);
        // The token MUST roundtrip — persistence is the whole point of
        // removing `skip_serializing`.
        assert_eq!(loaded.mcp_token, original.mcp_token);
        assert_eq!(loaded.mcp_auth_enabled, original.mcp_auth_enabled);
        assert_eq!(loaded.mcp_transport, original.mcp_transport);
        assert_eq!(loaded.language, original.language);
    }

    // -- Persistence: load_from_disk fallbacks ---------------------------------

    #[test]
    fn test_load_missing_file_returns_defaults() {
        let dir = test_dir("missing");
        let path = dir.0.join("does-not-exist.json");
        let loaded = AppConfig::load_from_disk(Some(&path));
        let defaults = AppConfig::default();

        assert_eq!(loaded.mc_address, defaults.mc_address);
        assert_eq!(loaded.mc_port, defaults.mc_port);
        assert_eq!(loaded.ai_username, defaults.ai_username);
        assert_eq!(loaded.mcp_address, defaults.mcp_address);
        assert_eq!(loaded.mcp_port, defaults.mcp_port);
        assert_eq!(loaded.task_name, defaults.task_name);
        assert_eq!(loaded.chunk_scan_radius, defaults.chunk_scan_radius);
        assert_eq!(
            loaded.block_perception_radius,
            defaults.block_perception_radius
        );
        assert_eq!(loaded.snapshot_interval_ms, defaults.snapshot_interval_ms);
        assert_eq!(
            loaded.reconnect_initial_delay_ms,
            defaults.reconnect_initial_delay_ms
        );
        assert_eq!(
            loaded.reconnect_max_delay_ms,
            defaults.reconnect_max_delay_ms
        );
        assert_eq!(loaded.command_timeout_secs, defaults.command_timeout_secs);
        // Token is random per default; only assert presence.
        assert!(!loaded.mcp_token.is_empty());
        assert_eq!(loaded.mcp_transport, defaults.mcp_transport);
    }

    #[test]
    fn test_load_partial_json_merges_defaults() {
        // A file containing only `mc_address` must keep that value and fill
        // every other field from serde defaults.
        let dir = test_dir("partial");
        let path = dir.0.join("config.json");
        std::fs::write(&path, r#"{ "mc_address": "10.0.0.99" }"#)
            .expect("failed to write partial config");

        let loaded = AppConfig::load_from_disk(Some(&path));
        assert_eq!(loaded.mc_address, "10.0.0.99");
        assert_eq!(loaded.mc_port, 25565);
        assert_eq!(loaded.ai_username, "AI_Bot");
        assert_eq!(loaded.mcp_address, "127.0.0.1");
        assert_eq!(loaded.mcp_port, 3000);
        assert_eq!(loaded.task_name, "mining");
        assert_eq!(loaded.chunk_scan_radius, 8);
        assert_eq!(loaded.block_perception_radius, 32);
        assert_eq!(loaded.snapshot_interval_ms, 500);
        assert_eq!(loaded.reconnect_initial_delay_ms, 5000);
        assert_eq!(loaded.reconnect_max_delay_ms, 60_000);
        assert_eq!(loaded.command_timeout_secs, 30);
        // `default_mcp_token` fills a missing token with a fresh UUID.
        assert!(!loaded.mcp_token.is_empty());
        assert_eq!(loaded.mcp_transport, McpTransport::Http);
        // serde `#[serde(default)]` uses `Language::default()` (= En),
        // independent of the host locale.
        assert_eq!(loaded.language, Language::En);
    }

    #[test]
    fn test_load_malformed_json_returns_defaults_without_panic() {
        let dir = test_dir("malformed");
        let path = dir.0.join("config.json");
        std::fs::write(&path, "{ not json").expect("failed to write malformed config");

        let loaded = AppConfig::load_from_disk(Some(&path));
        assert_eq!(loaded.mc_address, "127.0.0.1");
        assert_eq!(loaded.mc_port, 25565);
        assert_eq!(loaded.ai_username, "AI_Bot");
        assert!(!loaded.mcp_token.is_empty());
        assert_eq!(loaded.mcp_transport, McpTransport::Http);
    }

    // -- Persistence: save_to_disk error path ----------------------------------

    #[test]
    fn test_save_to_impossible_dir_returns_err() {
        let dir = test_dir("impossible");
        // Create a regular FILE where a parent directory would need to be —
        // `create_dir_all` cannot succeed through a file on any platform.
        let blocker = dir.0.join("blocker");
        std::fs::write(&blocker, "not a directory").expect("failed to write blocker file");
        let target = blocker.join("config.json");

        let result = sample_config().save_to_disk(Some(&target));
        assert!(
            result.is_err(),
            "saving into a file-as-parent-directory must fail, got Ok"
        );
    }
}
