//! UI internationalisation (i18n) for the Minecraft MCP server desktop UI.
//!
//! # Design
//!
//! The i18n layer is intentionally **functional**: there is no macro, no
//! `gettext`-style runtime string parsing, and no trait-based message bundle.
//! Instead each language lives in its own `.rs` file (`en.rs`, `zh_cn.rs`)
//! that exposes a single `lookup(key: TextKey) -> &'static str` function.
//! The lookup is a plain `match`, exhaustive per language with a catch-all
//! fallback to [`key_name`] so `tr` can **never panic**.
//!
//! The currently-active language is stored in a process-wide [`RwLock`]
//! (`static CURRENT`).  Reads are cheap (a short-lived read lock); writes
//! only happen when the user picks a new language in the settings panel.
//!
//! Public surface:
//! - [`Language`] — selectable UI language enum (serialisable so it can be
//!   persisted in [`AppConfig`](crate::config::AppConfig)).
//! - [`TextKey`] — the set of translatable UI strings.
//! - [`current`] / [`set`] — read or change the active language at runtime.
//! - [`tr`] — translate a [`TextKey`] using the active language.
//! - [`key_name`] — the variant name of a [`TextKey`] (used as the fallback
//!   for unknown keys inside each language's `lookup`).
//!
//! Adding a new language is a four-step process:
//! 1. Add a variant to [`Language`] (and update [`Language::default`] if you
//!    want to change the default).
//! 2. Create `xx.rs` with a `lookup` function matching the same arm order as
//!    [`en::lookup`].
//! 3. Add `pub mod xx;` below.
//! 4. Add a dispatch arm in [`tr`].

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::RwLock;

pub mod en;
pub mod zh_cn;

// ════════════════════════════════════════════════════════════════════
// Language — the active UI language, persisted in AppConfig
// ════════════════════════════════════════════════════════════════════

/// Selectable UI display language.
///
/// Persisted inside [`AppConfig`](crate::config::AppConfig) so the user's
/// choice survives restarts.  `Default` is [`Language::En`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
pub enum Language {
    /// English (US) — the canonical source strings in [`en::lookup`].
    #[default]
    En,
    /// Simplified Chinese (简体中文) — see [`zh_cn::lookup`].
    ZhCn,
}

// ════════════════════════════════════════════════════════════════════
// TextKey — the set of translatable UI strings
// ════════════════════════════════════════════════════════════════════

/// A translatable UI string identifier.
///
/// Each variant maps to exactly one English string in [`en::lookup`] and one
/// Simplified Chinese string in [`zh_cn::lookup`].  Translate via [`tr`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextKey {
    // ── Top-level / section headings ───────────────────────────────
    /// Window title and main heading shown at the top of the UI.
    AppTitle,
    /// Collapsible "Settings" panel heading.
    Settings,
    /// Collapsible "Status" panel heading.
    Status,
    /// Collapsible "MCP Config" panel heading.
    McpConfig,

    // ── Minecraft server section ───────────────────────────────────
    /// "Minecraft Server" section header.
    MinecraftServer,
    /// "Address:" field label.
    Address,
    /// "Port:" field label.
    Port,

    // ── Bot identity section ───────────────────────────────────────
    /// "Bot Identity" section header.
    BotIdentity,
    /// "Username:" field label.
    Username,

    // ── MCP server section ─────────────────────────────────────────
    /// "MCP Server" section header.
    McpServer,
    /// "Bind Address:" field label.
    BindAddress,
    /// "Bind Port:" field label.
    BindPort,
    /// "Transport:" field label.
    Transport,
    /// "HTTP (remote)" transport option label.
    TransportHttp,
    /// "Stdio (subprocess)" transport option label.
    TransportStdio,
    /// "Token:" field label.
    Token,
    /// "Bearer token for HTTP auth" hint text.
    TokenHint,

    // ── Scanning section ───────────────────────────────────────────
    /// "Scanning" section header.
    Scanning,
    /// "Chunk Scan Radius (1-16):" field label.
    ChunkScanRadius,
    /// "Block Perception Radius (8-64):" field label.
    BlockPerceptionRadius,
    /// "Snapshot Interval (ms):" field label.
    SnapshotInterval,

    // ── Timing section ─────────────────────────────────────────────
    /// "Timing" section header.
    Timing,
    /// "Reconnect Initial Delay (ms):" field label.
    ReconnectInitialDelay,
    /// "Reconnect Max Delay (ms):" field label.
    ReconnectMaxDelay,
    /// "Command Timeout (s):" field label.
    CommandTimeout,

    // ── Connect / Disconnect buttons ───────────────────────────────
    /// "Connect" button label.
    Connect,
    /// "Disconnect" button label.
    Disconnect,

    // ── Connection state badges ────────────────────────────────────
    /// "● Connected" coloured label.
    Connected,
    /// "● Connecting..." coloured label.
    Connecting,
    /// "● Disconnected" coloured label.
    Disconnected,

    // ── Status panel ───────────────────────────────────────────────
    /// "⚠ Error:" prefix for the error banner.
    Error,
    /// "Connection:" label.
    Connection,
    /// "● Online" coloured label.
    Online,
    /// "● Offline" coloured label.
    Offline,
    /// "Uptime:" label.
    Uptime,
    /// "s" unit suffix for uptime.
    UnitSeconds,

    // ── Player info section ────────────────────────────────────────
    /// "Player Info" collapsible header.
    PlayerInfo,
    /// "UUID:" label.
    Uuid,
    /// "Position:" label.
    Position,
    /// "Health:" label.
    Health,
    /// "Hunger:" label.
    Hunger,
    /// "Gamemode:" label.
    Gamemode,
    /// "Held Slot:" label.
    HeldSlot,

    // ── Nearby stats section ───────────────────────────────────────
    /// "Nearby Stats" collapsible header.
    NearbyStats,
    /// "Blocks in view:" label.
    BlocksInView,
    /// "Entities in view:" label.
    EntitiesInView,
    /// "Chunks loaded:" label.
    ChunksLoaded,
    /// "Loaded chunks:" label (list heading).
    LoadedChunks,
    /// "chunk" unit noun inside the chunk list.
    Chunk,

    // ── Command stats section ──────────────────────────────────────
    /// "Command Stats" collapsible header.
    CommandStats,
    /// "Commands processed:" label.
    CommandsProcessed,
    /// "Succeeded:" label.
    Succeeded,
    /// "Failed:" label.
    Failed,
    /// "Success rate:" label.
    SuccessRate,

    // ── Chat log section ───────────────────────────────────────────
    /// "Chat Log (last 10)" collapsible header.
    ChatLog,
    /// "No chat messages received yet." placeholder.
    NoChatMessages,

    // ── MCP config panel ───────────────────────────────────────────
    /// "Copy" button label.
    Copy,
    /// "Copy this JSON into your MCP client config (Claude Desktop / Cursor):" hint.
    CopyHint,

    // ── Language picker ────────────────────────────────────────────
    /// "Language:" picker label.
    Language,
    /// "English" selectable value.
    LangEn,
    /// "简体中文" selectable value.
    LangZhCn,
}

// ════════════════════════════════════════════════════════════════════
// Process-wide active language (RwLock so concurrent reads are cheap)
// ════════════════════════════════════════════════════════════════════

/// The currently-active UI language.
///
/// Initialised to [`Language::En`]; updated via [`set`] when the user picks
/// a new language in the settings panel (or when the persisted
/// [`AppConfig`](crate::config::AppConfig) is loaded at startup).
static CURRENT: RwLock<Language> = RwLock::new(Language::En);

/// Read the currently-active UI language.
///
/// Acquires a short-lived read lock on [`CURRENT`].  Cheap to call every
/// frame from the egui render loop.
pub fn current() -> Language {
    // The lock can only be poisoned if a writer panicked while holding it;
    // since `set` only writes a `Copy` enum, that is effectively impossible.
    // Recover by falling back to the default language rather than panicking
    // the UI thread.
    *CURRENT.read().unwrap_or_else(|e| e.into_inner())
}

/// Set the currently-active UI language.
///
/// Acquires a short-lived write lock on [`CURRENT`].  Subsequent calls to
/// [`tr`] will use the new language.
pub fn set(lang: Language) {
    // Poisoning recovery: a previously-panicked writer would leave the lock
    // poisoned; overwrite the inner value rather than crashing the UI.
    let mut guard = CURRENT.write().unwrap_or_else(|e| e.into_inner());
    *guard = lang;
}

// ════════════════════════════════════════════════════════════════════
// Translation entry points
// ════════════════════════════════════════════════════════════════════

/// Translate `key` using the currently-active language.
///
/// Dispatches to [`en::lookup`] or [`zh_cn::lookup`].  Each language's
/// `lookup` is total (uses a catch-all `_ =>` arm that calls [`key_name`]),
/// so this function can **never panic** — even if a new [`TextKey`] variant
/// is added before every language file is updated.
pub fn tr(key: TextKey) -> &'static str {
    match current() {
        Language::En => en::lookup(key),
        Language::ZhCn => zh_cn::lookup(key),
    }
}

/// Return the [`TextKey`] variant name as a static string.
///
/// Used as the fallback inside each language's `lookup` (so a missing
/// translation surfaces the key name rather than panicking) and useful for
/// debugging which key failed to translate.
pub fn key_name(key: TextKey) -> &'static str {
    match key {
        TextKey::AppTitle => "AppTitle",
        TextKey::Settings => "Settings",
        TextKey::Status => "Status",
        TextKey::McpConfig => "McpConfig",
        TextKey::MinecraftServer => "MinecraftServer",
        TextKey::Address => "Address",
        TextKey::Port => "Port",
        TextKey::BotIdentity => "BotIdentity",
        TextKey::Username => "Username",
        TextKey::McpServer => "McpServer",
        TextKey::BindAddress => "BindAddress",
        TextKey::BindPort => "BindPort",
        TextKey::Transport => "Transport",
        TextKey::TransportHttp => "TransportHttp",
        TextKey::TransportStdio => "TransportStdio",
        TextKey::Token => "Token",
        TextKey::TokenHint => "TokenHint",
        TextKey::Scanning => "Scanning",
        TextKey::ChunkScanRadius => "ChunkScanRadius",
        TextKey::BlockPerceptionRadius => "BlockPerceptionRadius",
        TextKey::SnapshotInterval => "SnapshotInterval",
        TextKey::Timing => "Timing",
        TextKey::ReconnectInitialDelay => "ReconnectInitialDelay",
        TextKey::ReconnectMaxDelay => "ReconnectMaxDelay",
        TextKey::CommandTimeout => "CommandTimeout",
        TextKey::Connect => "Connect",
        TextKey::Disconnect => "Disconnect",
        TextKey::Connected => "Connected",
        TextKey::Connecting => "Connecting",
        TextKey::Disconnected => "Disconnected",
        TextKey::Error => "Error",
        TextKey::Connection => "Connection",
        TextKey::Online => "Online",
        TextKey::Offline => "Offline",
        TextKey::Uptime => "Uptime",
        TextKey::UnitSeconds => "UnitSeconds",
        TextKey::PlayerInfo => "PlayerInfo",
        TextKey::Uuid => "Uuid",
        TextKey::Position => "Position",
        TextKey::Health => "Health",
        TextKey::Hunger => "Hunger",
        TextKey::Gamemode => "Gamemode",
        TextKey::HeldSlot => "HeldSlot",
        TextKey::NearbyStats => "NearbyStats",
        TextKey::BlocksInView => "BlocksInView",
        TextKey::EntitiesInView => "EntitiesInView",
        TextKey::ChunksLoaded => "ChunksLoaded",
        TextKey::LoadedChunks => "LoadedChunks",
        TextKey::Chunk => "Chunk",
        TextKey::CommandStats => "CommandStats",
        TextKey::CommandsProcessed => "CommandsProcessed",
        TextKey::Succeeded => "Succeeded",
        TextKey::Failed => "Failed",
        TextKey::SuccessRate => "SuccessRate",
        TextKey::ChatLog => "ChatLog",
        TextKey::NoChatMessages => "NoChatMessages",
        TextKey::Copy => "Copy",
        TextKey::CopyHint => "CopyHint",
        TextKey::Language => "Language",
        TextKey::LangEn => "LangEn",
        TextKey::LangZhCn => "LangZhCn",
    }
}

// ════════════════════════════════════════════════════════════════════
// Tests
// ════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    /// `tr` returns the English string by default (the `CURRENT` initial
    /// value), and `current()` reflects that initial state.
    #[test]
    fn test_tr_returns_english_by_default() {
        // Force a known starting state in case another test mutated CURRENT.
        set(Language::En);
        assert_eq!(current(), Language::En);
        assert_eq!(tr(TextKey::AppTitle), "Minecraft MCP Server");
        assert_eq!(tr(TextKey::Settings), "Settings");
        assert_eq!(tr(TextKey::Connect), "Connect");
    }

    /// After `set(ZhCn)`, `tr` returns the Simplified Chinese string and
    /// `current()` reflects the change.
    #[test]
    fn test_set_zh_cn_changes_tr_output() {
        set(Language::ZhCn);
        assert_eq!(current(), Language::ZhCn);
        assert_eq!(tr(TextKey::AppTitle), "Minecraft MCP 服务器");
        assert_eq!(tr(TextKey::Settings), "设置");
        assert_eq!(tr(TextKey::Connect), "连接");
    }

    /// Switching back to `En` restores the English strings.
    #[test]
    fn test_set_en_reverts_to_english() {
        set(Language::ZhCn);
        assert_eq!(tr(TextKey::AppTitle), "Minecraft MCP 服务器");
        set(Language::En);
        assert_eq!(current(), Language::En);
        assert_eq!(tr(TextKey::AppTitle), "Minecraft MCP Server");
    }

    /// `current()` reflects whatever was last passed to `set()`.
    #[test]
    fn test_current_reflects_set() {
        set(Language::ZhCn);
        assert_eq!(current(), Language::ZhCn);
        set(Language::En);
        assert_eq!(current(), Language::En);
    }

    /// `key_name` returns the variant name for every `TextKey`.
    #[test]
    fn test_key_name_returns_variant_name() {
        assert_eq!(key_name(TextKey::AppTitle), "AppTitle");
        assert_eq!(key_name(TextKey::ChunkScanRadius), "ChunkScanRadius");
        assert_eq!(key_name(TextKey::LangZhCn), "LangZhCn");
        // Spot-check a couple more to ensure the match is exhaustive.
        assert_eq!(key_name(TextKey::Error), "Error");
        assert_eq!(key_name(TextKey::UnitSeconds), "UnitSeconds");
    }

    /// `Language::default()` is `En` (so a fresh [`AppConfig`](crate::config::AppConfig)
    /// boots in English).
    #[test]
    fn test_language_default_is_english() {
        assert_eq!(Language::default(), Language::En);
    }
}
