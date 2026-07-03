//! Configuration types for the Minecraft MCP server.
//!
//! Provides [`AppConfig`] for UI-facing settings and [`RunStats`] for
//! thread-safe command tracking counters.  No file I/O — LAN servers
//! change ports each session.

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
    pub mc_address: String,
    /// Minecraft server port (default: `25565`).
    pub mc_port: u16,
    /// Bot in-game username (default: `"AI_Bot"`).
    pub ai_username: String,
    /// MCP server bind address (default: `"127.0.0.1"`).
    pub mcp_address: String,
    /// MCP server bind port (default: `3000`).
    pub mcp_port: u16,
    /// Descriptive task name for the UI (default: `"mining"`).
    pub task_name: String,
    /// How many chunks to scan around the player (range: 1–16, default: 8).
    pub chunk_scan_radius: u8,
    /// Block perception radius in blocks (range: 8–64, default: 32).
    pub block_perception_radius: u8,
    /// Interval between world snapshots in milliseconds (default: 500).
    pub snapshot_interval_ms: u64,
    /// Initial reconnect delay in milliseconds (default: 5000).
    pub reconnect_initial_delay_ms: u64,
    /// Maximum reconnect delay in milliseconds (default: 60000).
    pub reconnect_max_delay_ms: u64,
    /// Timeout for bot commands in seconds (default: 30).
    pub command_timeout_secs: u64,
    /// Authentication token presented by MCP clients over HTTP
    /// (default: `"minecraft-mcp-rs"`).
    #[serde(default = "default_mcp_token")]
    pub mcp_token: String,
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
    "minecraft-mcp-rs".to_string()
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            mc_address: "127.0.0.1".into(),
            mc_port: 25565,
            ai_username: "AI_Bot".into(),
            mcp_address: "127.0.0.1".into(),
            mcp_port: 3000,
            task_name: "mining".into(),
            chunk_scan_radius: 8,
            block_perception_radius: 32,
            snapshot_interval_ms: 500,
            reconnect_initial_delay_ms: 5000,
            reconnect_max_delay_ms: 60_000,
            command_timeout_secs: 30,
            mcp_token: default_mcp_token(),
            mcp_transport: McpTransport::default(),
            language: Language::default(),
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
        if self.command_timeout_secs == 0 {
            return Err("command_timeout_secs must be greater than 0".into());
        }
        if self.mcp_token.is_empty() {
            return Err("mcp_token must not be empty".into());
        }
        Ok(())
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
    }

    // -- McpTransport / mcp_token defaults ----------------------------------

    #[test]
    fn test_default_config() {
        let config = AppConfig::default();
        assert_eq!(config.mcp_token, "minecraft-mcp-rs");
        assert_eq!(config.mcp_transport, McpTransport::Http);
    }

    #[test]
    fn test_mcp_transport_default_is_http() {
        assert_eq!(McpTransport::default(), McpTransport::Http);
    }

    // -- Language field -----------------------------------------------------

    #[test]
    fn test_default_config_language() {
        let config = AppConfig::default();
        assert_eq!(config.language, Language::En);
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
        let mut config = AppConfig::default();
        config.mcp_token.clear();
        let err = config.validate().unwrap_err();
        assert!(err.contains("mcp_token"), "got: {err}");
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
}
