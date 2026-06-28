//! English (US) source strings for the UI i18n layer.
//!
//! This is the **canonical** language: every [`TextKey`](super::TextKey)
//! variant maps to a literal here, and the catch-all arm falls back to
//! [`super::key_name`] so the function is total even if a new variant is
//! added before this file is updated.
//!
//! Strings include any trailing punctuation (colons, ellipses) so they
//! render identically to the pre-i18n English UI.

use super::TextKey;

/// Translate `key` to English.
///
/// Returns a static English string.  The catch-all arm guarantees totality:
/// any unrecognised key surfaces its variant name rather than panicking.
pub(crate) fn lookup(key: TextKey) -> &'static str {
    #[allow(unreachable_patterns)] // keep catch-all for future TextKey variants
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
        TextKey::ChatLog => "Chat Log (last 10)",
        TextKey::NoChatMessages => "No chat messages received yet.",

        // ── MCP config panel ───────────────────────────────────────────
        TextKey::Copy => "Copy",
        TextKey::CopyHint => {
            "Copy this JSON into your MCP client config (Claude Desktop / Cursor):"
        }

        // ── Language picker ────────────────────────────────────────────
        TextKey::Language => "Language:",
        TextKey::LangEn => "English",
        TextKey::LangZhCn => "简体中文",

        // ── Catch-all: never panic, surface the variant name ───────────
        _ => super::key_name(key),
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

    /// The catch-all arm is reachable for variants that exist on the enum
    /// but were accidentally omitted from the match — simulate by passing
    /// every defined variant; none should panic.
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
            TextKey::Scanning,
            TextKey::ChunkScanRadius,
            TextKey::BlockPerceptionRadius,
            TextKey::SnapshotInterval,
            TextKey::Timing,
            TextKey::ReconnectInitialDelay,
            TextKey::ReconnectMaxDelay,
            TextKey::CommandTimeout,
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
            TextKey::Language,
            TextKey::LangEn,
            TextKey::LangZhCn,
        ];
        for k in all {
            assert!(!lookup(k).is_empty(), "lookup returned empty for {k:?}");
        }
    }
}
