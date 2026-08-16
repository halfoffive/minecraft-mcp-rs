//! Configuration types for the Minecraft MCP server.
//!
//! Provides [`AppConfig`] for UI-facing settings and [`RunStats`] for
//! thread-safe command tracking counters.
//!
//! # Configuration source: environment variables
//!
//! Configuration is read **exclusively from environment variables**
//! (12-factor style, cargo-style) — there is no config file anymore:
//!
//! - [`AppConfig::from_env`] starts from [`AppConfig::default()`] and
//!   overrides each field from its `MINECRAFT_MCP_*` environment variable
//!   when present (see the method docs for the full mapping).
//! - Malformed variable values log a warning and keep the default — startup
//!   never fails because of a typo in an environment variable.
//! - `MINECRAFT_MCP_TOKEN` is the ONLY way to pin the MCP bearer token;
//!   without it a fresh random UUID is generated per process start.
//! - Settings changed at runtime (UI panel, `update_settings` MCP tool)
//!   affect only the running process; restart with the environment variables
//!   to persist them.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicU64;
use std::time::Instant;

use crate::i18n::Language;

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
    /// Timeout for `fly_to` long-distance flights in seconds (default: 60).
    ///
    /// `command_timeout_secs` is too tight for long flights; this knob lets
    /// fly_to breathe without loosening every other command's timeout.
    #[serde(default = "default_fly_timeout_secs")]
    pub fly_timeout_secs: u64,
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
    /// UI display language (default: the host system locale via
    /// [`Language::from_system_locale`], matching [`AppConfig::default`]).
    ///
    /// L-30: the serde default previously drifted to [`Language::En`] while
    /// `AppConfig::default()` followed the system locale — deserializing an
    /// old config on a Chinese system silently got English.
    #[serde(default = "Language::from_system_locale")]
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

/// Serde default for [`AppConfig::fly_timeout_secs`].
fn default_fly_timeout_secs() -> u64 {
    60
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
            fly_timeout_secs: default_fly_timeout_secs(),
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
        if self.mcp_port == 0 {
            return Err("mcp_port must not be 0".into());
        }
        if self.mcp_address != "localhost" && !valid_bind_address(&self.mcp_address) {
            return Err(
                "mcp_address must be a valid IP address (e.g. 127.0.0.1 or 0.0.0.0) or \"localhost\""
                    .into(),
            );
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
        if self.fly_timeout_secs == 0 {
            return Err("fly_timeout_secs must be greater than 0".into());
        }
        if self.mcp_auth_enabled && self.mcp_token.is_empty() {
            return Err("mcp_token must not be empty when auth is enabled".into());
        }
        Ok(())
    }

    /// Load configuration from environment variables (12-factor, cargo-style).
    ///
    /// Starts from [`AppConfig::default()`] and overrides each field from
    /// its `MINECRAFT_MCP_*` environment variable when present:
    ///
    /// | Field | Environment variable |
    /// |-------|----------------------|
    /// | `mc_address` | `MINECRAFT_MCP_MC_ADDRESS` |
    /// | `mc_port` | `MINECRAFT_MCP_MC_PORT` |
    /// | `ai_username` | `MINECRAFT_MCP_AI_USERNAME` |
    /// | `mcp_address` | `MINECRAFT_MCP_MCP_ADDRESS` |
    /// | `mcp_port` | `MINECRAFT_MCP_MCP_PORT` |
    /// | `task_name` | `MINECRAFT_MCP_TASK_NAME` |
    /// | `chunk_scan_radius` | `MINECRAFT_MCP_CHUNK_SCAN_RADIUS` |
    /// | `block_perception_radius` | `MINECRAFT_MCP_BLOCK_PERCEPTION_RADIUS` |
    /// | `snapshot_interval_ms` | `MINECRAFT_MCP_SNAPSHOT_INTERVAL_MS` |
    /// | `reconnect_initial_delay_ms` | `MINECRAFT_MCP_RECONNECT_INITIAL_DELAY_MS` |
    /// | `reconnect_max_delay_ms` | `MINECRAFT_MCP_RECONNECT_MAX_DELAY_MS` |
    /// | `command_timeout_secs` | `MINECRAFT_MCP_COMMAND_TIMEOUT_SECS` |
    /// | `fly_timeout_secs` | `MINECRAFT_MCP_FLY_TIMEOUT_SECS` |
    /// | `mcp_token` | `MINECRAFT_MCP_TOKEN` |
    /// | `mcp_auth_enabled` | `MINECRAFT_MCP_AUTH_ENABLED` (`true`/`false`) |
    /// | `mcp_transport` | `MINECRAFT_MCP_TRANSPORT` (`stdio`/`http`) |
    /// | `language` | `MINECRAFT_MCP_LANGUAGE` (`en`/`zh_cn`) |
    ///
    /// Fallbacks (never panics):
    /// - variable unset → default value
    /// - variable unparsable → `tracing::warn!` + default value
    ///
    /// Like the old file loader this deliberately does NOT call
    /// [`AppConfig::validate`] — a stale environment must not prevent the
    /// app from starting; validation happens where settings are applied.
    pub fn from_env() -> AppConfig {
        let mut config = AppConfig::default();
        config.mc_address =
            env_parse_or_validated("MINECRAFT_MCP_MC_ADDRESS", config.mc_address, |s| {
                if s.is_empty() {
                    Err("must not be empty".into())
                } else {
                    Ok(())
                }
            });
        config.mc_port = env_parse_or_validated("MINECRAFT_MCP_MC_PORT", config.mc_port, |v| {
            if *v == 0 {
                Err("must be greater than 0".into())
            } else {
                Ok(())
            }
        });
        config.ai_username =
            env_parse_or_validated("MINECRAFT_MCP_AI_USERNAME", config.ai_username, |s| {
                if s.is_empty() {
                    Err("must not be empty".into())
                } else {
                    Ok(())
                }
            });
        // L-11: a bad bind address used to survive `from_env` and only trip
        // `main.rs`'s final validate(), which then discarded the ENTIRE env
        // config. It is now rejected per-field (same rule as validate())
        // with a warning, keeping every other variable intact.
        config.mcp_address =
            env_parse_or_validated("MINECRAFT_MCP_MCP_ADDRESS", config.mcp_address, |s| {
                if valid_bind_address(s) {
                    Ok(())
                } else {
                    Err("must be a valid IP address or \"localhost\"".into())
                }
            });
        config.mcp_port = env_parse_or_validated("MINECRAFT_MCP_MCP_PORT", config.mcp_port, |v| {
            if *v == 0 {
                Err("must be greater than 0".into())
            } else {
                Ok(())
            }
        });
        config.task_name = env_var_or("MINECRAFT_MCP_TASK_NAME", config.task_name);
        config.chunk_scan_radius = env_parse_or_validated(
            "MINECRAFT_MCP_CHUNK_SCAN_RADIUS",
            config.chunk_scan_radius,
            |v| {
                if !(1..=16).contains(v) {
                    Err(format!("must be between 1 and 16, got {v}"))
                } else {
                    Ok(())
                }
            },
        );
        config.block_perception_radius = env_parse_or_validated(
            "MINECRAFT_MCP_BLOCK_PERCEPTION_RADIUS",
            config.block_perception_radius,
            |v| {
                if !(8..=64).contains(v) {
                    Err(format!("must be between 8 and 64, got {v}"))
                } else {
                    Ok(())
                }
            },
        );
        config.snapshot_interval_ms = env_parse_or_validated(
            "MINECRAFT_MCP_SNAPSHOT_INTERVAL_MS",
            config.snapshot_interval_ms,
            |v| {
                if *v == 0 {
                    Err("must be greater than 0".into())
                } else {
                    Ok(())
                }
            },
        );
        config.reconnect_initial_delay_ms = env_parse_or_validated(
            "MINECRAFT_MCP_RECONNECT_INITIAL_DELAY_MS",
            config.reconnect_initial_delay_ms,
            |v| {
                if *v == 0 {
                    Err("must be greater than 0".into())
                } else {
                    Ok(())
                }
            },
        );
        config.reconnect_max_delay_ms = env_parse_or_validated(
            "MINECRAFT_MCP_RECONNECT_MAX_DELAY_MS",
            config.reconnect_max_delay_ms,
            |v| {
                if *v == 0 {
                    Err("must be greater than 0".into())
                } else {
                    Ok(())
                }
            },
        );
        config.command_timeout_secs = env_parse_or_validated(
            "MINECRAFT_MCP_COMMAND_TIMEOUT_SECS",
            config.command_timeout_secs,
            |v| {
                if *v == 0 {
                    Err("must be greater than 0".into())
                } else {
                    Ok(())
                }
            },
        );
        config.fly_timeout_secs = env_parse_or_validated(
            "MINECRAFT_MCP_FLY_TIMEOUT_SECS",
            config.fly_timeout_secs,
            |v| {
                if *v == 0 {
                    Err("must be greater than 0".into())
                } else {
                    Ok(())
                }
            },
        );
        config.mcp_token = env_var_or("MINECRAFT_MCP_TOKEN", config.mcp_token);
        config.mcp_auth_enabled =
            env_parse_or("MINECRAFT_MCP_AUTH_ENABLED", config.mcp_auth_enabled);
        if let Ok(v) = std::env::var("MINECRAFT_MCP_TRANSPORT") {
            if v.eq_ignore_ascii_case("stdio") {
                config.mcp_transport = McpTransport::Stdio;
            } else if v.eq_ignore_ascii_case("http") {
                config.mcp_transport = McpTransport::Http;
            } else {
                tracing::warn!(
                    name = "MINECRAFT_MCP_TRANSPORT",
                    value = %v,
                    "invalid transport, using default"
                );
            }
        }
        if let Ok(v) = std::env::var("MINECRAFT_MCP_LANGUAGE") {
            if v.eq_ignore_ascii_case("en") {
                config.language = Language::En;
            } else if v.eq_ignore_ascii_case("zh_cn") {
                config.language = Language::ZhCn;
            } else {
                tracing::warn!(
                    name = "MINECRAFT_MCP_LANGUAGE",
                    value = %v,
                    "invalid language, using default"
                );
            }
        }
        config
    }
}

// ---------------------------------------------------------------------------
// Environment-variable helpers
// ---------------------------------------------------------------------------

/// Is `address` acceptable for the MCP HTTP bind address?
///
/// Mirrors the rule [`AppConfig::validate`] applies (L-11 extracted the
/// predicate so `from_env`'s per-field check and the final validation gate
/// can never drift): `localhost` or a parseable [`std::net::IpAddr`].
fn valid_bind_address(address: &str) -> bool {
    address == "localhost" || address.parse::<std::net::IpAddr>().is_ok()
}

/// Read a `String`-typed env var, falling back to `fallback` when unset.
fn env_var_or(name: &str, fallback: String) -> String {
    std::env::var(name).unwrap_or(fallback)
}

/// Parse a numeric/bool env var, warning + falling back on parse failure.
fn env_parse_or<T: std::str::FromStr>(name: &str, fallback: T) -> T {
    match std::env::var(name) {
        Ok(v) => match v.parse::<T>() {
            Ok(parsed) => parsed,
            Err(_) => {
                tracing::warn!(
                    name,
                    value = %v,
                    "invalid value for environment variable; using default"
                );
                fallback
            }
        },
        Err(_) => fallback,
    }
}

/// Parse a numeric env var, then apply a range/semantic validation.
///
/// Malformed values warn and keep the default, exactly like [`env_parse_or`].
/// Values that parse but are semantically invalid (e.g. `0` for a positive
/// duration or an out-of-range radius) also warn and keep the default, so a
/// stale environment can never wedge the process with `snapshot_interval_ms=0`
/// or `command_timeout_secs=0`.
fn env_parse_or_validated<T, F>(name: &str, fallback: T, validate: F) -> T
where
    T: std::str::FromStr,
    F: FnOnce(&T) -> Result<(), String>,
{
    match std::env::var(name) {
        Ok(v) => match v.parse::<T>() {
            Ok(parsed) => match validate(&parsed) {
                Ok(()) => parsed,
                Err(reason) => {
                    tracing::warn!(
                        name,
                        value = %v,
                        %reason,
                        "invalid value for environment variable; using default"
                    );
                    fallback
                }
            },
            Err(_) => {
                tracing::warn!(
                    name,
                    value = %v,
                    "invalid value for environment variable; using default"
                );
                fallback
            }
        },
        Err(_) => fallback,
    }
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
        assert_eq!(config.fly_timeout_secs, 60);
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
        //
        // L-30: the serde default now matches `AppConfig::default()` (the
        // system locale) instead of a hardcoded `En` — rewritten to assert
        // the aligned behaviour so it is robust on Chinese systems too.
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
        assert_eq!(config.language, Language::from_system_locale());
        assert_eq!(config.language, AppConfig::default().language);
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

    // -- Environment loading ------------------------------------------------

    /// Serialises tests that mutate process environment variables: cargo runs
    /// tests in parallel threads, and `std::env::set_var` races between them.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// RAII guard: saves the previous value of every touched variable and
    /// restores it on drop, so env tests never leak state into each other.
    struct EnvGuard {
        saved: Vec<(&'static str, Option<String>)>,
    }

    impl EnvGuard {
        fn set(name: &'static str, value: &str) -> Self {
            let saved = vec![(name, std::env::var(name).ok())];
            // `std::env::set_var` is `unsafe` since the Rust 2024 edition.
            unsafe { std::env::set_var(name, value) };
            EnvGuard { saved }
        }

        fn remove(name: &'static str) -> Self {
            let saved = vec![(name, std::env::var(name).ok())];
            unsafe { std::env::remove_var(name) };
            EnvGuard { saved }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (name, prev) in &self.saved {
                match prev {
                    Some(v) => unsafe { std::env::set_var(name, v) },
                    None => unsafe { std::env::remove_var(name) },
                }
            }
        }
    }

    /// A config with deterministic, `validate()`-passing values, used to
    /// assert that `from_env` maps every variable to its field.
    fn env_sample() -> AppConfig {
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
        config.fly_timeout_secs = 90;
        config.mcp_token = "env-token-123".into();
        config.mcp_auth_enabled = true;
        config.mcp_transport = McpTransport::Stdio;
        config.language = Language::ZhCn;
        config
    }

    /// Set every `MINECRAFT_MCP_*` variable to the [`env_sample`] values.
    fn set_all_env_vars() -> Vec<EnvGuard> {
        vec![
            EnvGuard::set("MINECRAFT_MCP_MC_ADDRESS", "10.0.0.5"),
            EnvGuard::set("MINECRAFT_MCP_MC_PORT", "25566"),
            EnvGuard::set("MINECRAFT_MCP_AI_USERNAME", "TestBot"),
            EnvGuard::set("MINECRAFT_MCP_MCP_ADDRESS", "127.0.0.1"),
            EnvGuard::set("MINECRAFT_MCP_MCP_PORT", "3001"),
            EnvGuard::set("MINECRAFT_MCP_TASK_NAME", "testing"),
            EnvGuard::set("MINECRAFT_MCP_CHUNK_SCAN_RADIUS", "4"),
            EnvGuard::set("MINECRAFT_MCP_BLOCK_PERCEPTION_RADIUS", "16"),
            EnvGuard::set("MINECRAFT_MCP_SNAPSHOT_INTERVAL_MS", "250"),
            EnvGuard::set("MINECRAFT_MCP_RECONNECT_INITIAL_DELAY_MS", "1000"),
            EnvGuard::set("MINECRAFT_MCP_RECONNECT_MAX_DELAY_MS", "30000"),
            EnvGuard::set("MINECRAFT_MCP_COMMAND_TIMEOUT_SECS", "10"),
            EnvGuard::set("MINECRAFT_MCP_FLY_TIMEOUT_SECS", "90"),
            EnvGuard::set("MINECRAFT_MCP_TOKEN", "env-token-123"),
            EnvGuard::set("MINECRAFT_MCP_AUTH_ENABLED", "true"),
            EnvGuard::set("MINECRAFT_MCP_TRANSPORT", "stdio"),
            EnvGuard::set("MINECRAFT_MCP_LANGUAGE", "zh_cn"),
        ]
    }

    /// Remove every `MINECRAFT_MCP_*` variable.
    fn clear_all_env_vars() -> Vec<EnvGuard> {
        [
            "MINECRAFT_MCP_MC_ADDRESS",
            "MINECRAFT_MCP_MC_PORT",
            "MINECRAFT_MCP_AI_USERNAME",
            "MINECRAFT_MCP_MCP_ADDRESS",
            "MINECRAFT_MCP_MCP_PORT",
            "MINECRAFT_MCP_TASK_NAME",
            "MINECRAFT_MCP_CHUNK_SCAN_RADIUS",
            "MINECRAFT_MCP_BLOCK_PERCEPTION_RADIUS",
            "MINECRAFT_MCP_SNAPSHOT_INTERVAL_MS",
            "MINECRAFT_MCP_RECONNECT_INITIAL_DELAY_MS",
            "MINECRAFT_MCP_RECONNECT_MAX_DELAY_MS",
            "MINECRAFT_MCP_COMMAND_TIMEOUT_SECS",
            "MINECRAFT_MCP_FLY_TIMEOUT_SECS",
            "MINECRAFT_MCP_TOKEN",
            "MINECRAFT_MCP_AUTH_ENABLED",
            "MINECRAFT_MCP_TRANSPORT",
            "MINECRAFT_MCP_LANGUAGE",
        ]
        .into_iter()
        .map(EnvGuard::remove)
        .collect()
    }

    #[test]
    fn test_from_env_without_vars_returns_defaults() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _guards = clear_all_env_vars();
        let config = AppConfig::from_env();
        let defaults = AppConfig::default();
        assert_eq!(config.mc_address, defaults.mc_address);
        assert_eq!(config.mc_port, defaults.mc_port);
        assert_eq!(config.ai_username, defaults.ai_username);
        assert_eq!(config.mcp_address, defaults.mcp_address);
        assert_eq!(config.mcp_port, defaults.mcp_port);
        assert_eq!(config.task_name, defaults.task_name);
        assert_eq!(config.chunk_scan_radius, defaults.chunk_scan_radius);
        assert_eq!(
            config.block_perception_radius,
            defaults.block_perception_radius
        );
        assert_eq!(config.snapshot_interval_ms, defaults.snapshot_interval_ms);
        assert_eq!(
            config.reconnect_initial_delay_ms,
            defaults.reconnect_initial_delay_ms
        );
        assert_eq!(
            config.reconnect_max_delay_ms,
            defaults.reconnect_max_delay_ms
        );
        assert_eq!(config.command_timeout_secs, defaults.command_timeout_secs);
        assert_eq!(config.fly_timeout_secs, defaults.fly_timeout_secs);
        assert!(!config.mcp_token.is_empty());
        assert_eq!(config.mcp_transport, defaults.mcp_transport);
        assert_eq!(config.language, defaults.language);
    }

    #[test]
    fn test_from_env_overrides_every_field() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _guards = set_all_env_vars();
        let config = AppConfig::from_env();
        let expected = env_sample();
        assert_eq!(config.mc_address, expected.mc_address);
        assert_eq!(config.mc_port, expected.mc_port);
        assert_eq!(config.ai_username, expected.ai_username);
        assert_eq!(config.mcp_address, expected.mcp_address);
        assert_eq!(config.mcp_port, expected.mcp_port);
        assert_eq!(config.task_name, expected.task_name);
        assert_eq!(config.chunk_scan_radius, expected.chunk_scan_radius);
        assert_eq!(
            config.block_perception_radius,
            expected.block_perception_radius
        );
        assert_eq!(config.snapshot_interval_ms, expected.snapshot_interval_ms);
        assert_eq!(
            config.reconnect_initial_delay_ms,
            expected.reconnect_initial_delay_ms
        );
        assert_eq!(
            config.reconnect_max_delay_ms,
            expected.reconnect_max_delay_ms
        );
        assert_eq!(config.command_timeout_secs, expected.command_timeout_secs);
        assert_eq!(config.fly_timeout_secs, expected.fly_timeout_secs);
        assert_eq!(config.mcp_token, expected.mcp_token);
        assert_eq!(config.mcp_auth_enabled, expected.mcp_auth_enabled);
        assert_eq!(config.mcp_transport, expected.mcp_transport);
        assert_eq!(config.language, expected.language);
    }

    #[test]
    fn test_from_env_invalid_values_fall_back_without_panic() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _guards = [
            EnvGuard::set("MINECRAFT_MCP_MC_PORT", "not-a-port"),
            EnvGuard::set("MINECRAFT_MCP_CHUNK_SCAN_RADIUS", "99999"),
            EnvGuard::set("MINECRAFT_MCP_AUTH_ENABLED", "maybe"),
            EnvGuard::set("MINECRAFT_MCP_TRANSPORT", "carrier-pigeon"),
            EnvGuard::set("MINECRAFT_MCP_LANGUAGE", "klingon"),
        ];
        let config = AppConfig::from_env();
        let defaults = AppConfig::default();
        assert_eq!(config.mc_port, defaults.mc_port);
        assert_eq!(config.chunk_scan_radius, defaults.chunk_scan_radius);
        assert_eq!(config.mcp_auth_enabled, defaults.mcp_auth_enabled);
        assert_eq!(config.mcp_transport, defaults.mcp_transport);
        assert_eq!(config.language, defaults.language);
    }

    #[test]
    fn test_from_env_zero_values_fall_back_without_panic() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _guards = [
            EnvGuard::set("MINECRAFT_MCP_MC_PORT", "0"),
            EnvGuard::set("MINECRAFT_MCP_MCP_PORT", "0"),
            EnvGuard::set("MINECRAFT_MCP_CHUNK_SCAN_RADIUS", "0"),
            EnvGuard::set("MINECRAFT_MCP_BLOCK_PERCEPTION_RADIUS", "0"),
            EnvGuard::set("MINECRAFT_MCP_SNAPSHOT_INTERVAL_MS", "0"),
            EnvGuard::set("MINECRAFT_MCP_RECONNECT_INITIAL_DELAY_MS", "0"),
            EnvGuard::set("MINECRAFT_MCP_RECONNECT_MAX_DELAY_MS", "0"),
            EnvGuard::set("MINECRAFT_MCP_COMMAND_TIMEOUT_SECS", "0"),
            EnvGuard::set("MINECRAFT_MCP_FLY_TIMEOUT_SECS", "0"),
        ];
        let config = AppConfig::from_env();
        let defaults = AppConfig::default();
        assert_eq!(config.mc_port, defaults.mc_port);
        assert_eq!(config.mcp_port, defaults.mcp_port);
        assert_eq!(config.chunk_scan_radius, defaults.chunk_scan_radius);
        assert_eq!(
            config.block_perception_radius,
            defaults.block_perception_radius
        );
        assert_eq!(config.snapshot_interval_ms, defaults.snapshot_interval_ms);
        assert_eq!(
            config.reconnect_initial_delay_ms,
            defaults.reconnect_initial_delay_ms
        );
        assert_eq!(
            config.reconnect_max_delay_ms,
            defaults.reconnect_max_delay_ms
        );
        assert_eq!(config.command_timeout_secs, defaults.command_timeout_secs);
        assert_eq!(config.fly_timeout_secs, defaults.fly_timeout_secs);
        // The resulting config must pass full validation too.
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_from_env_transport_case_insensitive() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _guards = [EnvGuard::set("MINECRAFT_MCP_TRANSPORT", "STDIO")];
        let config = AppConfig::from_env();
        assert_eq!(config.mcp_transport, McpTransport::Stdio);
    }

    #[test]
    fn test_validate_fly_timeout_zero() {
        let mut config = AppConfig::default();
        config.fly_timeout_secs = 0;
        let err = config.validate().unwrap_err();
        assert!(err.contains("fly_timeout_secs"), "got: {err}");
    }

    // -- L-30: language serde default vs AppConfig::default -------------------

    /// A config missing the `language` field must deserialize to the SAME
    /// value as [`AppConfig::default`] — which follows the system locale —
    /// not a hardcoded [`Language::En`]. The `#[serde(default)]` on
    /// `language` previously diverged from `AppConfig::default()`.
    #[test]
    fn test_deserialized_absent_language_equals_default_language() {
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
        assert_eq!(config.language, AppConfig::default().language);
        assert_eq!(config.language, Language::from_system_locale());
    }

    // -- L-11: per-field string env validation --------------------------------

    /// L-11: a semantically invalid string env var (bad `mcp_address`) must
    /// be rejected PER-FIELD inside `from_env`, keeping the OTHER variables
    /// and never degrading the whole config to defaults — which is what
    /// happened before, because the bad value survived `from_env` and only
    /// `main.rs`'s final `validate()` (replacing the ENTIRE config with
    /// defaults) caught it.
    #[test]
    fn test_from_env_bad_mcp_address_keeps_other_variables() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _guards = [
            EnvGuard::set("MINECRAFT_MCP_MCP_ADDRESS", "not-an-ip"),
            EnvGuard::set("MINECRAFT_MCP_MC_PORT", "25566"),
        ];
        let config = AppConfig::from_env();
        assert_eq!(config.mc_port, 25566, "valid sibling variable must survive");
        assert_eq!(
            config.mcp_address,
            AppConfig::default().mcp_address,
            "bad mcp_address must fall back to the default"
        );
        assert!(
            config.validate().is_ok(),
            "per-field fallback must leave a fully valid config"
        );
    }

    /// Empty `mc_address` / `ai_username` are rejected per-field (mirroring
    /// [`AppConfig::validate`]) and fall back to their defaults.
    #[test]
    fn test_from_env_empty_mc_address_falls_back() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _guards = [
            EnvGuard::set("MINECRAFT_MCP_MC_ADDRESS", ""),
            EnvGuard::set("MINECRAFT_MCP_AI_USERNAME", ""),
        ];
        let config = AppConfig::from_env();
        assert_eq!(config.mc_address, AppConfig::default().mc_address);
        assert_eq!(config.ai_username, AppConfig::default().ai_username);
        assert!(config.validate().is_ok());
    }
}
