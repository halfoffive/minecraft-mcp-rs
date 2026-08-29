//! English (US) source strings for the UI i18n layer.
//!
//! This is the **canonical** language: every [`TextKey`]
//! variant maps to a literal here. The match is exhaustive with **no
//! catch-all arm** — adding a [`TextKey`] variant fails to COMPILE until it
//! is translated in this file (the compile-time completeness guard).
//!
//! Strings include any trailing punctuation (colons, ellipses) so they
//! render identically to the pre-i18n English UI.

use super::TextKey;

/// Translate `key` to English.
///
/// Returns a static English string. The match is exhaustive — a new
/// [`TextKey`] variant must be added here (and in `zh_cn::lookup`) before
/// the crate compiles.
pub(crate) fn lookup(key: TextKey) -> &'static str {
    // compile-time exhaustiveness guard: no catch-all
    match key {
        // ── Top-level / section headings ───────────────────────────────
        TextKey::AppTitle => "Minecraft MCP Server",
        TextKey::Settings => "Settings",
        TextKey::Status => "Status",
        TextKey::McpConfig => "MCP Config",

        // ── Minecraft server section ───────────────────────────────────
        TextKey::MinecraftServer => "Minecraft Server",
        TextKey::Address => "Address:",
        TextKey::Port => "Port:",

        // ── Bot identity section ───────────────────────────────────────
        TextKey::BotIdentity => "Bot Identity",
        TextKey::Username => "Username:",

        // ── MCP server section ─────────────────────────────────────────
        TextKey::McpServer => "MCP Server",
        TextKey::BindAddress => "Bind Address:",
        TextKey::BindPort => "Bind Port:",
        TextKey::Transport => "Transport:",
        TextKey::TransportHttp => "HTTP (remote)",
        TextKey::TransportStdio => "Stdio (subprocess)",
        TextKey::Token => "Token:",
        TextKey::TokenHint => "Bearer token for HTTP auth",
        TextKey::RequireToken => "Require Bearer token (HTTP):",

        // ── MCP server status / warnings ───────────────────────────────
        TextKey::McpServerLabel => "MCP:",
        TextKey::McpServerRunning => "Running on {addr}",
        TextKey::McpServerStdio => "Running on stdio",
        TextKey::McpServerFailed => "Failed: {msg}",
        TextKey::McpServerStopped => "Stopped",
        TextKey::TlsWarning => "⚠ No TLS — use trusted network or reverse proxy",

        // ── Scanning section ───────────────────────────────────────────
        TextKey::Scanning => "Scanning",
        TextKey::ChunkScanRadius => "Chunk Scan Radius (1-16):",
        TextKey::BlockPerceptionRadius => "Block Perception Radius (8-64):",
        TextKey::SnapshotInterval => "Snapshot Interval (ms):",

        // ── Timing section ─────────────────────────────────────────────
        TextKey::Timing => "Timing",
        TextKey::ReconnectInitialDelay => "Reconnect Initial Delay (ms):",
        TextKey::ReconnectMaxDelay => "Reconnect Max Delay (ms):",
        TextKey::CommandTimeout => "Command Timeout (s):",
        TextKey::FlyTimeout => "Fly Timeout (s):",

        // ── Connect / Disconnect buttons ───────────────────────────────
        TextKey::Connect => "Connect",
        TextKey::Disconnect => "Disconnect",

        // ── Connection state badges ────────────────────────────────────
        TextKey::Connected => "● Connected",
        TextKey::Connecting => "● Connecting...",
        TextKey::Disconnected => "● Disconnected",

        // ── Status panel ───────────────────────────────────────────────
        TextKey::Error => "⚠ Error:",
        TextKey::Connection => "Connection:",
        TextKey::Online => "● Online",
        TextKey::Offline => "● Offline",
        TextKey::Uptime => "Uptime:",
        TextKey::UnitSeconds => "s",

        // ── Player info section ────────────────────────────────────────
        TextKey::PlayerInfo => "Player Info",
        TextKey::Uuid => "UUID:",
        TextKey::Position => "Position:",
        TextKey::Health => "Health:",
        TextKey::Hunger => "Hunger:",
        TextKey::Gamemode => "Gamemode:",
        TextKey::HeldSlot => "Held Slot:",

        // ── Nearby stats section ───────────────────────────────────────
        TextKey::NearbyStats => "Nearby Stats",
        TextKey::BlocksInView => "Blocks in view:",
        TextKey::EntitiesInView => "Entities in view:",
        TextKey::ChunksLoaded => "Chunks loaded:",
        TextKey::LoadedChunks => "Loaded chunks:",
        TextKey::Chunk => "chunk",

        // ── Command stats section ──────────────────────────────────────
        TextKey::CommandStats => "Command Stats",
        TextKey::CommandsProcessed => "Commands processed:",
        TextKey::Succeeded => "Succeeded:",
        TextKey::Failed => "Failed:",
        TextKey::SuccessRate => "Success rate:",

        // ── Chat log section ───────────────────────────────────────────
        TextKey::ChatLog => "Chat Log (last 50)",
        TextKey::NoChatMessages => "No chat messages received yet.",

        // ── MCP config panel ───────────────────────────────────────────
        TextKey::Copy => "Copy",
        TextKey::CopyHint => {
            "Copy this JSON into your MCP client config (Claude Desktop / Cursor):"
        }
        TextKey::NpxConfig => "npm / npx (no Rust toolchain needed):",
        TextKey::BunxConfig => "bunx (Bun runtime — no Rust toolchain needed):",

        // ── Language picker ────────────────────────────────────────────
        TextKey::Language => "Language:",
        TextKey::LangEn => "English",
        TextKey::LangZhCn => "简体中文",

        // ── World view preview panel ───────────────────────────────────
        TextKey::Preview => "World View Preview",
        TextKey::WorldView => "World View:",
        TextKey::Refresh => "Refresh",
        TextKey::RefreshTooltip => "Re-render the current snapshot at radius=8, scale=2",
        TextKey::ConfigPendingHint => {
            "Some settings have un-applied edits — the JSON below may differ from the running config until you click Connect."
        }
        TextKey::WorldViewPlaceholder => {
            "No render cached yet — click Refresh to render the current snapshot."
        }
    }
}

// ════════════════════════════════════════════════════════════════════
// Tests
// ════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    /// Spot-check that several well-known keys return their canonical
    /// English values.
    #[test]
    fn test_lookup_returns_canonical_english() {
        assert_eq!(lookup(TextKey::AppTitle), "Minecraft MCP Server");
        assert_eq!(lookup(TextKey::Settings), "Settings");
        assert_eq!(lookup(TextKey::TransportHttp), "HTTP (remote)");
        assert_eq!(lookup(TextKey::Connected), "● Connected");
        assert_eq!(lookup(TextKey::LangEn), "English");
    }

    /// Every defined variant resolves to a non-empty string. (Exhaustiveness
    /// itself is enforced at compile time by the missing catch-all arm; this
    /// test additionally guards against accidentally translating a key to an
    /// empty string.)
    #[test]
    fn test_lookup_total_for_all_variants() {
        // Iterate over every TextKey we know about.  This is a hand-rolled
        // list (TextKey doesn't derive a real iterator), but it covers all
        // variants and ensures each resolves to *some* non-empty string.
        let all = [
            TextKey::AppTitle,
            TextKey::Settings,
            TextKey::Status,
            TextKey::McpConfig,
            TextKey::MinecraftServer,
            TextKey::Address,
            TextKey::Port,
            TextKey::BotIdentity,
            TextKey::Username,
            TextKey::McpServer,
            TextKey::BindAddress,
            TextKey::BindPort,
            TextKey::Transport,
            TextKey::TransportHttp,
            TextKey::TransportStdio,
            TextKey::Token,
            TextKey::TokenHint,
            TextKey::RequireToken,
            TextKey::McpServerLabel,
            TextKey::McpServerRunning,
            TextKey::McpServerStdio,
            TextKey::McpServerFailed,
            TextKey::McpServerStopped,
            TextKey::TlsWarning,
            TextKey::Scanning,
            TextKey::ChunkScanRadius,
            TextKey::BlockPerceptionRadius,
            TextKey::SnapshotInterval,
            TextKey::Timing,
            TextKey::ReconnectInitialDelay,
            TextKey::ReconnectMaxDelay,
            TextKey::CommandTimeout,
            TextKey::FlyTimeout,
            TextKey::Connect,
            TextKey::Disconnect,
            TextKey::Connected,
            TextKey::Connecting,
            TextKey::Disconnected,
            TextKey::Error,
            TextKey::Connection,
            TextKey::Online,
            TextKey::Offline,
            TextKey::Uptime,
            TextKey::UnitSeconds,
            TextKey::PlayerInfo,
            TextKey::Uuid,
            TextKey::Position,
            TextKey::Health,
            TextKey::Hunger,
            TextKey::Gamemode,
            TextKey::HeldSlot,
            TextKey::NearbyStats,
            TextKey::BlocksInView,
            TextKey::EntitiesInView,
            TextKey::ChunksLoaded,
            TextKey::LoadedChunks,
            TextKey::Chunk,
            TextKey::CommandStats,
            TextKey::CommandsProcessed,
            TextKey::Succeeded,
            TextKey::Failed,
            TextKey::SuccessRate,
            TextKey::ChatLog,
            TextKey::NoChatMessages,
            TextKey::Copy,
            TextKey::CopyHint,
            TextKey::NpxConfig,
            TextKey::BunxConfig,
            TextKey::Language,
            TextKey::LangEn,
            TextKey::LangZhCn,
            TextKey::Preview,
            TextKey::WorldView,
            TextKey::Refresh,
            TextKey::RefreshTooltip,
            TextKey::WorldViewPlaceholder,
            // 2026-08-30 review: ConfigPendingHint (the 77th key) was
            // missing from this array, so the "every lookup non-empty"
            // invariant silently skipped it.
            TextKey::ConfigPendingHint,
        ];
        for k in all {
            assert!(!lookup(k).is_empty(), "lookup returned empty for {k:?}");
        }
    }
}
