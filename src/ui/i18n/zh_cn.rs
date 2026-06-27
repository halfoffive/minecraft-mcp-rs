//! Simplified Chinese (简体中文) translations for the UI i18n layer.
//!
//! Mirrors [`super::en::lookup`] arm-for-arm with Simplified Chinese values.
//! The catch-all arm falls back to [`super::key_name`] so the function is
//! total even if a new variant is added before this file is updated.
//!
//! Punctuation follows Simplified Chinese conventions (full-width colons
//! `：`, full-width ellipsis `…`); the dot/bullet prefixes on connection
//! badges are kept as-is so they line up with the English UI.

use super::TextKey;

/// Translate `key` to Simplified Chinese.
///
/// Returns a static Chinese string.  The catch-all arm guarantees totality:
/// any unrecognised key surfaces its variant name rather than panicking.
pub(crate) fn lookup(key: TextKey) -> &'static str {
    #[allow(unreachable_patterns)] // keep catch-all for future TextKey variants
    match key {
        // ── Top-level / section headings ───────────────────────────────
        TextKey::AppTitle => "Minecraft MCP 服务器",
        TextKey::Settings => "设置",
        TextKey::Status => "状态",
        TextKey::McpConfig => "MCP 配置",

        // ── Minecraft server section ───────────────────────────────────
        TextKey::MinecraftServer => "Minecraft 服务器",
        TextKey::Address => "地址：",
        TextKey::Port => "端口：",

        // ── Bot identity section ───────────────────────────────────────
        TextKey::BotIdentity => "机器人身份",
        TextKey::Username => "用户名：",

        // ── MCP server section ─────────────────────────────────────────
        TextKey::McpServer => "MCP 服务器",
        TextKey::BindAddress => "绑定地址：",
        TextKey::BindPort => "绑定端口：",
        TextKey::Transport => "传输方式：",
        TextKey::TransportHttp => "HTTP（远程）",
        TextKey::TransportStdio => "Stdio（子进程）",
        TextKey::Token => "令牌：",
        TextKey::TokenHint => "HTTP 认证的 Bearer 令牌",

        // ── Scanning section ───────────────────────────────────────────
        TextKey::Scanning => "扫描",
        TextKey::ChunkScanRadius => "区块扫描半径 (1-16)：",
        TextKey::BlockPerceptionRadius => "方块感知半径 (8-64)：",
        TextKey::SnapshotInterval => "快照间隔 (毫秒)：",

        // ── Timing section ─────────────────────────────────────────────
        TextKey::Timing => "时序",
        TextKey::ReconnectInitialDelay => "重连初始延迟 (毫秒)：",
        TextKey::ReconnectMaxDelay => "重连最大延迟 (毫秒)：",
        TextKey::CommandTimeout => "命令超时 (秒)：",

        // ── Connect / Disconnect buttons ───────────────────────────────
        TextKey::Connect => "连接",
        TextKey::Disconnect => "断开",

        // ── Connection state badges ────────────────────────────────────
        TextKey::Connected => "● 已连接",
        TextKey::Connecting => "● 连接中…",
        TextKey::Disconnected => "● 已断开",

        // ── Status panel ───────────────────────────────────────────────
        TextKey::Error => "⚠ 错误：",
        TextKey::Connection => "连接：",
        TextKey::Online => "● 在线",
        TextKey::Offline => "● 离线",
        TextKey::Uptime => "运行时长：",
        TextKey::UnitSeconds => "秒",

        // ── Player info section ────────────────────────────────────────
        TextKey::PlayerInfo => "玩家信息",
        TextKey::Uuid => "UUID：",
        TextKey::Position => "位置：",
        TextKey::Health => "生命值：",
        TextKey::Hunger => "饥饿值：",
        TextKey::Gamemode => "游戏模式：",
        TextKey::HeldSlot => "手持栏位：",

        // ── Nearby stats section ───────────────────────────────────────
        TextKey::NearbyStats => "附近统计",
        TextKey::BlocksInView => "可见方块：",
        TextKey::EntitiesInView => "可见实体：",
        TextKey::ChunksLoaded => "已加载区块：",
        TextKey::LoadedChunks => "已加载区块列表：",
        TextKey::Chunk => "区块",

        // ── Command stats section ──────────────────────────────────────
        TextKey::CommandStats => "命令统计",
        TextKey::CommandsProcessed => "已处理命令：",
        TextKey::Succeeded => "成功：",
        TextKey::Failed => "失败：",
        TextKey::SuccessRate => "成功率：",

        // ── Chat log section ───────────────────────────────────────────
        TextKey::ChatLog => "聊天记录（最近 10 条）",
        TextKey::NoChatMessages => "暂未收到聊天消息。",

        // ── MCP config panel ───────────────────────────────────────────
        TextKey::Copy => "复制",
        TextKey::CopyHint => "将此 JSON 复制到你的 MCP 客户端配置中（Claude Desktop / Cursor）：",

        // ── Language picker ────────────────────────────────────────────
        TextKey::Language => "语言：",
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
    /// Simplified Chinese values.
    #[test]
    fn test_lookup_returns_canonical_chinese() {
        assert_eq!(lookup(TextKey::AppTitle), "Minecraft MCP 服务器");
        assert_eq!(lookup(TextKey::Settings), "设置");
        assert_eq!(lookup(TextKey::TransportHttp), "HTTP（远程）");
        assert_eq!(lookup(TextKey::Connected), "● 已连接");
        assert_eq!(lookup(TextKey::LangZhCn), "简体中文");
    }

    /// Every variant resolves to a non-empty string (no panic).
    #[test]
    fn test_lookup_total_for_all_variants() {
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
