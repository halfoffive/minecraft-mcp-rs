//! MCP query tools for reading world/entity/player state from SharedState.
//!
//! Each function checks `SharedState::is_online()` first: if the bot is
//! offline, `is_connected` returns `{"connected":false}` and all other
//! query tools return `{"error":"Bot is currently offline"}`.
//!
//! `get_self_info` / `get_inventory` accept `force=true` (default) to trigger
//! an immediate snapshot rebuild before reading, so an agent that just
//! dropped an item / moved / teleported sees the fresh state instead of a
//! 500 ms-stale snapshot.

use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use serde::Deserialize;
use serde_json::json;

use crate::channel::BotCommandSender;
use crate::command_validate::clamp_to_i32;
use crate::error::BotError;
use crate::state::SharedState;
use crate::types::{BotCommand, GameMode};

// ---------------------------------------------------------------------------
// Public query functions — called from the #[tool] methods in server.rs
// ---------------------------------------------------------------------------

/// Input for the `get_self_info` MCP tool.
#[derive(Deserialize, Default, rmcp::schemars::JsonSchema)]
pub struct SelfInfoInput {
    /// Force an immediate snapshot refresh before reading (default true).
    /// Set false to read the last cached snapshot without waiting.
    #[serde(default = "default_true")]
    pub force: bool,
}

/// Input for the `get_inventory` MCP tool.
#[derive(Deserialize, Default, rmcp::schemars::JsonSchema)]
pub struct InventoryInput {
    /// Force an immediate snapshot refresh before reading (default true).
    /// Set false to read the last cached snapshot without waiting.
    #[serde(default = "default_true")]
    pub force: bool,
}

/// Input for the `get_server_info` MCP tool.
#[derive(Deserialize, Default, rmcp::schemars::JsonSchema)]
pub struct ServerInfoInput {
    /// Re-run the live `/seed` command probe (default false). The probe result
    /// is cached until the next `refresh=true` call.
    #[serde(default)]
    pub refresh: bool,
}

/// Serde default: `force` defaults to `true` so existing callers (who never
/// passed the parameter) keep getting a fresh read after their action.
fn default_true() -> bool {
    true
}

/// Get information about the bot's own player (uuid, username, position,
/// health, hunger, gamemode, held item).
///
/// When `force` is true (the default) a snapshot rebuild is requested first
/// so the returned state reflects the most recent world changes rather than
/// the throttled 500 ms snapshot. Returns the serialized
/// [`crate::types::SelfPlayer`] as a JSON string, or an offline error.
pub async fn get_self_info(
    state: &Arc<SharedState>,
    input: SelfInfoInput,
) -> Result<String, BotError> {
    crate::mcp::common::require_online(state)?;
    if input.force {
        refresh_snapshot_and_wait(state).await;
    }
    let snapshot = state.read_snapshot();
    serde_json::to_string(&snapshot.self_player)
        .map_err(|e| BotError::Internal(format!("Serialization error: {e}")))
}

/// Get the bot's full player inventory.
///
/// When `force` is true (the default) a snapshot rebuild is requested first
/// so the returned inventory reflects the most recent container-content
/// packets rather than the throttled snapshot. Returns the 36 main slots as
/// an array of occupied slots (empty slots are omitted), plus the currently
/// selected hotbar slot.
pub async fn get_inventory(
    state: &Arc<SharedState>,
    input: InventoryInput,
) -> Result<String, BotError> {
    crate::mcp::common::require_online(state)?;
    if input.force {
        refresh_snapshot_and_wait(state).await;
    }
    let snapshot = state.read_snapshot();
    Ok(json!({
        "inventory": snapshot.self_player.inventory,
        "held_item_slot": snapshot.self_player.held_item_slot,
    })
    .to_string())
}

/// Request an immediate snapshot rebuild and wait (bounded) for it to land.
///
/// Best-effort: the wait is capped at [`SNAPSHOT_FORCE_WAIT`] so a dead or
/// stalled bot event loop cannot hang a query tool. On timeout (or when the
/// bot never processes the request) the caller reads the current snapshot
/// anyway — a fresh read of *something* beats hanging the tool.
pub(crate) async fn refresh_snapshot_and_wait(state: &Arc<SharedState>) {
    let rx = state.request_snapshot_refresh();
    let _ = tokio::time::timeout(SNAPSHOT_FORCE_WAIT, rx).await;
}

/// Bounded wait for a forced snapshot rebuild (see
/// [`refresh_snapshot_and_wait`]).
const SNAPSHOT_FORCE_WAIT: Duration = Duration::from_secs(3);

// ---------------------------------------------------------------------------
// Nearby query input structs
// ---------------------------------------------------------------------------

/// Input for the `get_nearby_blocks` MCP tool.
#[derive(Deserialize, Default, rmcp::schemars::JsonSchema)]
pub struct NearbyBlocksInput {
    /// Chebyshev (square) radius around the bot to search. Range: 1..=100.
    #[schemars(range(min = 1, max = 100))]
    pub radius: u32,
    /// Optional case-insensitive substring filter on block_type. If None or
    /// empty, all block types are returned.
    pub filter_type: Option<String>,
    /// Top-only mode: return only the highest NON-AIR block of each (x, z)
    /// column. Surface-oriented tasks (pathfinding, base building) should
    /// pass true — a flat world at radius 16 collapses from ~340 KB of
    /// stacked layers to a single surface layer.
    #[serde(default)]
    pub top_only: bool,
    /// Maximum number of blocks to return. The response reports
    /// `truncated: true` when the match count exceeds this cap (default 500).
    #[serde(default = "default_max_payload")]
    #[schemars(range(min = 1, max = 10000))]
    pub max_blocks: u32,
}

/// Serde default: cap the response at 500 blocks so a large radius can
/// never flood the LLM context (the historical 340 KB response). Shared by
/// `get_nearby_blocks` and `get_nearby_entities`.
fn default_max_payload() -> u32 {
    500
}

/// Input for the `get_nearby_entities` MCP tool.
#[derive(Deserialize, Default, rmcp::schemars::JsonSchema)]
pub struct NearbyEntitiesInput {
    /// Chebyshev (square) radius around the bot to search. Range: 1..=100.
    #[schemars(range(min = 1, max = 100))]
    pub radius: u32,
    /// Maximum number of entities to return. The response reports
    /// `truncated: true` when the match count exceeds this cap (default 500).
    #[serde(default = "default_max_payload")]
    #[schemars(range(min = 1, max = 10000))]
    pub max_entities: u32,
}

// ---------------------------------------------------------------------------
// get_nearby_blocks — single-entry response cache (L-17)
// ---------------------------------------------------------------------------

/// One cached [`get_nearby_blocks`] response: the request parameters plus the
/// snapshot revision it was computed from, and the serialized response.
///
/// The key includes `snapshot_seq` (monotonic, bumped on every
/// `update_snapshot` / `modify_snapshot`) rather than `timestamp`
/// (seconds-granularity, can repeat for two consecutive 500 ms builds).
struct NearbyBlocksCache {
    snapshot_seq: u64,
    radius: u32,
    /// The normalized (lowercased, empty→None) filter the response was
    /// computed with.
    filter_type: Option<String>,
    top_only: bool,
    max_blocks: u32,
    response: String,
}

/// File-local single-entry cache for [`get_nearby_blocks`].
///
/// `get_nearby_blocks` is hot (an LLM probing a scene re-queries it
/// repeatedly), and every call used to re-scan the whole snapshot (~230k
/// blocks at a large radius) plus an O(n log n) `top_only` sort — while
/// `get_world_view` already caches its render. Keyed on
/// `(snapshot_seq, radius, filter_type, top_only, max_blocks)` so a cache
/// hit is byte-identical to a fresh compute. Single entry keeps memory
/// bounded. Poisoning recovery per project convention.
static NEARBY_BLOCKS_CACHE: LazyLock<Mutex<Option<NearbyBlocksCache>>> =
    LazyLock::new(|| Mutex::new(None));

/// Get blocks near the bot within the given Chebyshev (square) radius.
///
/// If `filter_type` is `Some(ft)` and non-empty, only blocks whose
/// `block_type` contains `ft` (case-insensitive substring match) are
/// included.
///
/// `top_only` collapses each (x, z) column to its highest NON-AIR block;
/// `max_blocks` caps the response (with `truncated: true` reported when the
/// cap is hit) so a large radius can never flood the LLM context.
///
/// The response is an object: `{blocks, count, total_matched, truncated,
/// top_only}`.
pub fn get_nearby_blocks(
    state: &Arc<SharedState>,
    radius: u32,
    filter_type: Option<String>,
    top_only: bool,
    max_blocks: u32,
) -> Result<String, BotError> {
    if !(1..=100).contains(&radius) {
        return Err(BotError::InvalidParams(format!(
            "radius must be in range 1..=100, got {radius}"
        )));
    }
    if !(1..=10000).contains(&max_blocks) {
        return Err(BotError::InvalidParams(format!(
            "max_blocks must be in range 1..=10000, got {max_blocks}"
        )));
    }
    crate::mcp::common::require_online(state)?;
    let snapshot = state.read_snapshot();
    let center = snapshot.self_player.position;
    let r = clamp_to_i32(radius);

    // Normalize the filter once, before both the cache lookup and the scan:
    // empty behaves as None and matching is case-insensitive, so `None`,
    // `Some("")`, `Some("stone")` and `Some("Stone")` are the same request
    // and must share one cache entry. The per-block match uses the
    // non-allocating ASCII substring helper (block ids are pure ASCII).
    let ft_key: Option<String> = filter_type
        .as_deref()
        .filter(|ft| !ft.is_empty())
        .map(|ft| ft.to_lowercase());

    // L-17 cache hit: same snapshot revision + parameters → the stored
    // response is byte-identical to a fresh compute; skip the full-snapshot
    // scan and the top_only sort.
    {
        let cache = NEARBY_BLOCKS_CACHE
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = cache.as_ref()
            && entry.snapshot_seq == snapshot.snapshot_seq
            && entry.radius == radius
            && entry.filter_type == ft_key
            && entry.top_only == top_only
            && entry.max_blocks == max_blocks
        {
            tracing::trace!(
                snapshot_seq = snapshot.snapshot_seq,
                radius,
                "get_nearby_blocks cache hit"
            );
            return Ok(entry.response.clone());
        }
    }

    let mut matched: Vec<&crate::types::BlockEntry> = snapshot
        .blocks
        .iter()
        .filter(|b| {
            (b.position.x - center.x).abs() <= r
                && (b.position.y - center.y).abs() <= r
                && (b.position.z - center.z).abs() <= r
        })
        .filter(|b| match &ft_key {
            Some(ft) => crate::utils::contains_ascii_case_insensitive(&b.block_type, ft),
            None => true,
        })
        .collect();

    let total_matched = matched.len();

    if top_only {
        // Group by (x, z) and keep the entry with the greatest y.
        // `dedup_by` keeps the FIRST of each run, so sort with the highest
        // y first (Reverse) — the survivor of each column is its top block.
        matched.sort_by(|a, b| {
            (a.position.x, a.position.z, std::cmp::Reverse(a.position.y)).cmp(&(
                b.position.x,
                b.position.z,
                std::cmp::Reverse(b.position.y),
            ))
        });
        matched.dedup_by(|a, b| a.position.x == b.position.x && a.position.z == b.position.z);
    }

    // Cap the response; report truncation honestly so the caller can shrink
    // the radius / add a filter instead of reasoning about a partial list
    // that looks complete.
    let truncated = matched.len() > max_blocks as usize;
    if truncated {
        matched.truncate(max_blocks as usize);
    }

    let response = serde_json::json!({
        "blocks": matched,
        "count": matched.len(),
        "total_matched": total_matched,
        "truncated": truncated,
        "top_only": top_only,
    });
    let response_str = serde_json::to_string(&response)
        .map_err(|e| BotError::Internal(format!("Serialization error: {e}")))?;

    // L-17: store for the next identical call (single entry, last write
    // wins under concurrency — the response is deterministic per key).
    *NEARBY_BLOCKS_CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Some(NearbyBlocksCache {
        snapshot_seq: snapshot.snapshot_seq,
        radius,
        filter_type: ft_key,
        top_only,
        max_blocks,
        response: response_str.clone(),
    });

    Ok(response_str)
}

/// Get entities near the bot within the given Chebyshev (square) radius.
///
/// Convenience wrapper over [`get_nearby_entities_capped`] with the default
/// 500-entity payload cap (kept for callers that do not need to tune the
/// cap, e.g. integration tests).
pub fn get_nearby_entities(state: &Arc<SharedState>, radius: u32) -> Result<String, BotError> {
    get_nearby_entities_capped(state, radius, default_max_payload())
}

/// Get entities near the bot within the given Chebyshev (square) radius.
///
/// `max_entities` caps the response (with `truncated: true` reported when
/// the cap is hit) so a large radius can never flood the LLM context —
/// mirroring `get_nearby_blocks`' `max_blocks` handling (R-11).
///
/// The response is an object: `{entities, count, truncated}`.
pub fn get_nearby_entities_capped(
    state: &Arc<SharedState>,
    radius: u32,
    max_entities: u32,
) -> Result<String, BotError> {
    if !(1..=100).contains(&radius) {
        return Err(BotError::InvalidParams(format!(
            "radius must be in range 1..=100, got {radius}"
        )));
    }
    if !(1..=10000).contains(&max_entities) {
        return Err(BotError::InvalidParams(format!(
            "max_entities must be in range 1..=10000, got {max_entities}"
        )));
    }
    crate::mcp::common::require_online(state)?;
    let snapshot = state.read_snapshot();
    let center = snapshot.self_player.position;
    let r = clamp_to_i32(radius);

    let mut entities: Vec<&crate::types::EntityEntry> = snapshot
        .entities
        .iter()
        .filter(|e| {
            (e.position.x - center.x).abs() <= r
                && (e.position.y - center.y).abs() <= r
                && (e.position.z - center.z).abs() <= r
        })
        .collect();

    // Cap the response; report truncation honestly so the caller can shrink
    // the radius instead of reasoning about a partial list that looks
    // complete.
    let truncated = entities.len() > max_entities as usize;
    if truncated {
        entities.truncate(max_entities as usize);
    }

    let response = serde_json::json!({
        "entities": entities,
        "count": entities.len(),
        "truncated": truncated,
    });
    serde_json::to_string(&response)
        .map_err(|e| BotError::Internal(format!("Serialization error: {e}")))
}

/// Get a summary of chunks currently loaded around the bot.
///
/// Returns a JSON array of `(chunk_x, chunk_z)` tuples.
pub fn get_chunk_summary(state: &Arc<SharedState>) -> Result<String, BotError> {
    crate::mcp::common::require_online(state)?;
    let snapshot = state.read_snapshot();
    serde_json::to_string(&snapshot.chunk_summary)
        .map_err(|e| BotError::Internal(format!("Serialization error: {e}")))
}

/// Check whether the bot is currently connected to a Minecraft server.
///
/// Returns `{"connected":true}` or `{"connected":false}`.
pub fn is_connected(state: &Arc<SharedState>) -> Result<String, BotError> {
    Ok(json!({"connected": state.is_online()}).to_string())
}

// ---------------------------------------------------------------------------
// get_server_info — reports commands_enabled and current gamemode
// ---------------------------------------------------------------------------

/// Convert a [`GameMode`] to its lowercase string name for JSON output.
fn gamemode_to_str(mode: GameMode) -> &'static str {
    match mode {
        GameMode::Survival => "survival",
        GameMode::Creative => "creative",
        GameMode::Adventure => "adventure",
        GameMode::Spectator => "spectator",
    }
}

/// Report whether commands are enabled on the server and the current gamemode.
///
/// `commands_enabled` is probed live by sending `/seed` and watching the
/// server's chat reply: accepted → `true`, rejected (`CommandRejected`) →
/// `false`, unknown (probe timed out / not yet run) → the azalea
/// `PermissionLevel` heuristic or `null`. The probe result is cached in
/// [`SharedState`] until the next `refresh=true` call, and merged into every
/// snapshot build. This reflects "commands actually work here" — on cheat /
/// plugin servers a non-OP player can run commands, which the old
/// OP-level-only inference got wrong. `gamemode` is one of
/// `survival|creative|adventure|spectator`. `bot_busy` reports whether the
/// command executor is currently processing another command.
pub async fn get_server_info(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    input: ServerInfoInput,
) -> Result<String, BotError> {
    crate::mcp::common::require_online(state)?;

    // (Re)run the live probe when requested or when we have no cached result
    // at all. Sending the probe through the command channel keeps it serial
    // with other bot commands, so a busy executor simply queues it.
    if input.refresh || state.get_commands_probe().is_none() {
        probe_commands_enabled(state, sender).await;
    }

    // Read through a forced refresh so `commands_enabled` reflects the merged
    // probe value and `gamemode` is fresh.
    refresh_snapshot_and_wait(state).await;
    let snapshot = state.read_snapshot();
    Ok(json!({
        "commands_enabled": snapshot.commands_enabled,
        "gamemode": gamemode_to_str(snapshot.self_player.gamemode),
        "bot_busy": state.executor_busy(),
    })
    .to_string())
}

/// How long the `/seed` commands probe waits for the executor to reply.
///
/// The probe only needs to know whether the server accepted the command —
/// it must NOT block behind a busy executor for the full 30 s command
/// timeout: `give_item` / `get_server_info(refresh)` would otherwise stall
/// for half a minute whenever another command is running (L-18).
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// Run a live `/seed` command-availability probe and cache the result.
///
/// The single source of truth for "can this bot run commands": every tool
/// that gates on command availability (e.g. `give_item`) must call this and
/// trust the probe, not the `PermissionLevel` heuristic in the cached
/// snapshot — after a reconnect the permission component can lag behind the
/// real server state, and a stale `Some(false)` would otherwise reject
/// commands that actually work.
///
/// Probe outcome: `Some(true)` accepted, `Some(false)` rejected, `None`
/// unknown (timeout / offline mid-probe / no feedback — the previous cached
/// value is preserved). The result is cached in [`SharedState`] via
/// `set_commands_probe` and merged into every snapshot build. The probe
/// envelope is bounded at [`PROBE_TIMEOUT`], so a busy executor yields a
/// timeout that preserves the previous value instead of a 30 s stall.
pub(crate) async fn probe_commands_enabled(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
) -> Option<bool> {
    probe_commands_enabled_with_timeout(state, sender, PROBE_TIMEOUT).await
}

/// The probe logic with an explicit envelope timeout (unit-testable with a
/// short duration; production uses [`PROBE_TIMEOUT`]).
async fn probe_commands_enabled_with_timeout(
    state: &Arc<SharedState>,
    sender: &BotCommandSender,
    timeout: Duration,
) -> Option<bool> {
    let probe = match sender
        .send_command_with_timeout(BotCommand::ExecuteCommand("/seed".into()), timeout)
        .await
    {
        Ok(_) => Some(true),
        Err(BotError::CommandRejected { .. }) => Some(false),
        // Timeout/offline/internal: keep the previous value (or None).
        Err(_) => state.get_commands_probe(),
    };
    state.set_commands_probe(probe);
    probe
}

// ---------------------------------------------------------------------------
// get_hotbar — the 9 hotbar slots (the slot-order invariant lives here)
// ---------------------------------------------------------------------------

/// Input for the `get_hotbar` MCP tool.
#[derive(Deserialize, Default, rmcp::schemars::JsonSchema)]
pub struct HotbarInput {
    /// Force an immediate snapshot refresh before reading (default true).
    #[serde(default = "default_true")]
    pub force: bool,
}

/// Get the bot's 9 hotbar slots (0-8) plus the currently selected slot.
///
/// Returns every slot — occupied slots as `{"slot", "item_id", "count"}`,
/// empty slots as `null` — so a client can see exactly what the hotbar
/// holds before using `set_hotbar_item` / `equip_tool` / `drop_item`
/// (the shared root cause behind their historical slot mistakes). `force`
/// behaves like [`get_inventory`].
pub async fn get_hotbar(state: &Arc<SharedState>, input: HotbarInput) -> Result<String, BotError> {
    crate::mcp::common::require_online(state)?;
    if input.force {
        refresh_snapshot_and_wait(state).await;
    }
    let snapshot = state.read_snapshot();
    let slots: Vec<Option<serde_json::Value>> = (0..=8u8)
        .map(|slot| {
            snapshot
                .self_player
                .inventory
                .iter()
                .find(|entry| entry.slot_index == slot)
                .map(|entry| {
                    json!({
                        "slot": entry.slot_index,
                        "item_id": entry.item_id,
                        "count": entry.count,
                    })
                })
        })
        .collect();
    Ok(json!({
        "hotbar": slots,
        "held_item_slot": snapshot.self_player.held_item_slot,
    })
    .to_string())
}

// ---------------------------------------------------------------------------
// get_bot_status — cheap polling endpoint for long-running operations
// ---------------------------------------------------------------------------

/// Input for the `get_bot_status` MCP tool.
#[derive(Deserialize, Default, rmcp::schemars::JsonSchema)]
pub struct BotStatusInput {
    /// Force an immediate snapshot rebuild before reading (default false —
    /// this is the cheap polling endpoint, so it reads the cached snapshot).
    #[serde(default)]
    pub force: bool,
}

/// Lightweight status poll for long-running operations (`fly_to` / mining /
/// `collect_items`): connection state, whether the executor is busy, the
/// bot's block + precise position, heading, vitals, and snapshot age.
///
/// Unlike [`get_self_info`] this defaults to the cached snapshot (no forced
/// rebuild) and never errors while offline — it reports `connected: false`
/// so a poller can wait for a reconnect.
pub async fn get_bot_status(
    state: &Arc<SharedState>,
    input: BotStatusInput,
) -> Result<String, BotError> {
    if !state.is_online() {
        return Ok(json!({
            "connected": false,
            "bot_busy": false,
        })
        .to_string());
    }
    if input.force {
        refresh_snapshot_and_wait(state).await;
    }
    let snapshot = state.read_snapshot();
    let player = &snapshot.self_player;
    Ok(json!({
        "connected": true,
        "bot_busy": state.executor_busy(),
        "position": [player.position.x, player.position.y, player.position.z],
        "position_precise": player.position_precise,
        "yaw": player.yaw,
        "health": player.health,
        "hunger": player.hunger,
        "gamemode": gamemode_to_str(player.gamemode),
        "held_item_slot": player.held_item_slot,
        "snapshot_timestamp": snapshot.timestamp,
    })
    .to_string())
}

// ---------------------------------------------------------------------------
// get_world_view — top-down PNG render for multimodal models
// ---------------------------------------------------------------------------

/// Input for the `get_world_view` MCP tool.
#[derive(Deserialize, Default, rmcp::schemars::JsonSchema)]
pub struct GetWorldViewInput {
    /// Half-extent of the top-down view in blocks (1-32). The rendered image
    /// is `(2*radius+1) x (2*radius+1)` blocks.
    #[schemars(range(min = 1, max = 32))]
    pub radius: u8,
    /// Pixels per block (1/2/4/8). Higher values produce a larger image
    /// for multimodal LLMs that prefer higher-resolution input. `0` and
    /// unsupported values fall back to `1` (legacy 1-pixel-per-block
    /// render). Defaults to `1` so existing clients see no change.
    ///
    /// The image dimensions are `(2*radius+1) * scale` pixels per side —
    /// for example `radius=8, scale=4` produces a `(2*8+1) * 4 = 68x68`
    /// PNG.
    #[serde(default)]
    #[schemars(range(min = 1, max = 8))]
    pub scale: u8,
}

/// Render a top-down PNG of the world around the bot and return it as MCP
/// multi-content (image + text annotation).
///
/// The response is a `Vec<Content>` containing:
///
/// 1. The PNG image as base64 (`Content::Image`, mime `image/png`).
/// 2. A text annotation (`Content::Text`) with JSON metadata: centre
///    coordinates, radius, scale, yaw (if known), and snapshot timestamp.
///    Multimodal LLMs can read this side-by-side with the image to anchor
///    pixel coordinates back to world coordinates.
///
/// ## Caching
///
/// Before re-rendering, the function checks
/// [`SharedState::get_world_view_cache`]. If the cached entry's
/// `snapshot_timestamp`, `radius`, and `scale` all match the current
/// request, the cached PNG + annotation are returned without invoking
/// [`render_topdown_enhanced`](crate::mcp::render::render_topdown_enhanced)
/// again. This makes repeated `get_world_view` calls between snapshot
/// ticks effectively free.
///
/// Validates `radius` is in `1..=32` and the bot is online; on error
/// returns a [`BotError`] so rmcp converts it to a standard MCP error
/// response.
pub fn get_world_view(
    state: &Arc<SharedState>,
    radius: u8,
    scale: u8,
) -> Result<Vec<rmcp::model::Content>, BotError> {
    if !(1..=32).contains(&radius) {
        return Err(BotError::InvalidParams(format!(
            "radius must be 1-32, got {radius}"
        )));
    }
    // scale is clamped inside render_topdown_enhanced; we don't reject
    // out-of-range values here, just fall back to 1 (so existing clients
    // passing `scale=0` or omitting it entirely get the legacy behaviour).
    crate::mcp::common::require_online(state)?;

    // Clamp scale to a supported value (1/2/4/8). Invalid inputs fall
    // back to 1 so the annotation JSON accurately reflects what was
    // rendered — clients reading `scale: 0` would otherwise mis-render.
    let scale = if crate::mcp::render::VALID_SCALES.contains(&scale) {
        scale
    } else {
        crate::mcp::render::DEFAULT_SCALE
    };

    let snapshot = state.read_snapshot();
    let snapshot_ts = snapshot.timestamp;
    let snapshot_seq = snapshot.snapshot_seq;

    // Cache hit: same snapshot revision + radius + scale → return cached
    // bytes. `timestamp` alone is seconds-granularity and can repeat for two
    // consecutive 500 ms snapshot builds; `snapshot_seq` is monotonic.
    if let Some(cache) = state.get_world_view_cache()
        && cache.snapshot_seq == snapshot_seq
        && cache.radius == radius
        && cache.scale == scale
    {
        tracing::trace!(
            snapshot_seq,
            snapshot_ts,
            radius,
            scale,
            "get_world_view cache hit — returning cached PNG"
        );
        return Ok(vec![
            rmcp::model::Content::image(cache.png_base64, "image/png"),
            rmcp::model::Content::text(cache.annotation_json.clone()),
        ]);
    }

    // Cache miss: re-render.
    tracing::debug!(
        snapshot_seq,
        snapshot_ts,
        radius,
        scale,
        "get_world_view cache miss — re-rendering"
    );
    let (png_bytes, block_count, entity_count) =
        crate::mcp::render::render_topdown_enhanced(&snapshot, radius, scale)?;
    let encoded = crate::mcp::render::base64_encode(&png_bytes);

    // Build the JSON annotation. Carries enough metadata for a multimodal
    // LLM to anchor pixel coords back to world coords (centre, radius,
    // scale) and to know which way the bot is facing (yaw).
    let (center_x, center_y, center_z) = snapshot
        .self_player
        .position_precise
        .map(|p| (p[0], p[1], p[2]))
        .unwrap_or((
            snapshot.self_player.position.x as f64,
            snapshot.self_player.position.y as f64,
            snapshot.self_player.position.z as f64,
        ));
    let annotation = serde_json::json!({
        "center": [center_x, center_y, center_z],
        "radius": radius,
        "scale": scale,
        "yaw": snapshot.self_player.yaw,
        "snapshot_timestamp": snapshot_ts,
        "image_size": ((2 * radius as u32 + 1) * scale.max(1) as u32),
        // Counts are what the rendered image actually shows (distinct block
        // columns + entities inside `radius`), NOT the whole snapshot —
        // the snapshot holds hundreds of thousands of blocks far beyond
        // any viewport.
        "block_count": block_count,
        "entity_count": entity_count,
    });
    let annotation_json = annotation.to_string();

    // Store in cache for the next call.
    state.set_world_view_cache(crate::state::WorldViewCache {
        snapshot_seq,
        radius,
        scale,
        png_base64: encoded.clone(),
        block_count,
        entity_count,
        annotation_json: annotation_json.clone(),
    });

    Ok(vec![
        rmcp::model::Content::image(encoded, "image/png"),
        rmcp::model::Content::text(annotation_json),
    ])
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::types::{
        BlockEntry, BlockPos, BotResult, EntityEntry, GameMode, InventorySlot, SelfPlayer,
        WorldSnapshot,
    };
    use base64::Engine;

    // -- Helpers ---------------------------------------------------------------

    /// Create a SharedState with a rich snapshot already loaded.
    fn state_with_snapshot() -> Arc<SharedState> {
        let state = SharedState::new(AppConfig::default());
        state.set_online(true);

        let snap = WorldSnapshot {
            blocks: vec![
                BlockEntry {
                    position: BlockPos::new(0, 64, 0),
                    block_type: "stone".into(),
                    block_state: None,
                },
                BlockEntry {
                    position: BlockPos::new(0, 65, 0),
                    block_type: "dirt".into(),
                    block_state: None,
                },
                BlockEntry {
                    position: BlockPos::new(10, 64, 0),
                    block_type: "diamond_ore".into(),
                    block_state: None,
                },
            ],
            entities: vec![
                EntityEntry {
                    id: 1,
                    uuid: "e1".into(),
                    entity_type: "zombie".into(),
                    position: BlockPos::new(1, 64, 0),
                    display_name: Some("Zombie".into()),
                    health: Some(20.0),
                },
                EntityEntry {
                    id: 2,
                    uuid: "e2".into(),
                    entity_type: "creeper".into(),
                    position: BlockPos::new(100, 64, 0),
                    display_name: None,
                    health: Some(20.0),
                },
            ],
            self_player: SelfPlayer {
                uuid: "player-uuid".into(),
                username: "TestBot".into(),
                position: BlockPos::new(0, 64, 0),
                health: 18.0,
                hunger: 15,
                gamemode: GameMode::Survival,
                held_item_slot: 3,
                inventory: vec![
                    InventorySlot {
                        slot_index: 0,
                        item_id: "iron_pickaxe".into(),
                        count: 1,
                    },
                    InventorySlot {
                        slot_index: 1,
                        item_id: "oak_planks".into(),
                        count: 64,
                    },
                ],
                position_precise: None,
                yaw: None,
            },
            timestamp: 42,
            chunk_summary: vec![(0, 0), (-1, 0)],
            commands_enabled: None,
            ..Default::default()
        };
        state.update_snapshot(snap);
        Arc::new(state)
    }

    /// SharedState with the bot offline.
    fn offline_state() -> Arc<SharedState> {
        Arc::new(SharedState::new(AppConfig::default()))
    }

    // -- get_self_info ---------------------------------------------------------

    #[tokio::test]
    async fn test_get_self_info_online() {
        let state = state_with_snapshot();
        // force=false keeps the test fast — no forced rebuild is needed to
        // exercise the serialization path (the state-level tests cover the
        // force-refresh channel).
        let result = get_self_info(&state, SelfInfoInput { force: false })
            .await
            .unwrap();
        assert!(result.contains("TestBot"));
        assert!(result.contains("player-uuid"));
        assert!(result.contains("18.0")); // health
        assert!(result.contains("15")); // hunger
    }

    #[tokio::test]
    async fn test_get_self_info_offline() {
        let state = offline_state();
        let result = get_self_info(&state, SelfInfoInput { force: false }).await;
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    #[tokio::test]
    async fn test_get_self_info_force_true_reads_fresh_snapshot() {
        let state = state_with_snapshot();
        // force=true requests a rebuild then falls back to the current
        // snapshot when no bot loop answers (3s cap). Assert the fallback
        // still returns valid data and does not hang forever.
        let result = get_self_info(&state, SelfInfoInput { force: true })
            .await
            .unwrap();
        assert!(result.contains("TestBot"));
    }

    // -- get_inventory ---------------------------------------------------------

    #[tokio::test]
    async fn test_get_inventory_online() {
        let state = state_with_snapshot();
        let result = get_inventory(&state, InventoryInput { force: false })
            .await
            .unwrap();
        assert!(result.contains("held_item_slot"));
        assert!(result.contains('3'));
        assert!(result.contains("inventory"));
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        let inventory = parsed["inventory"].as_array().unwrap();
        assert_eq!(inventory.len(), 2);
        assert_eq!(inventory[0]["slot_index"], 0);
        assert_eq!(inventory[0]["item_id"], "iron_pickaxe");
        assert_eq!(inventory[0]["count"], 1);
        assert_eq!(inventory[1]["slot_index"], 1);
        assert_eq!(inventory[1]["item_id"], "oak_planks");
        assert_eq!(inventory[1]["count"], 64);
    }

    #[tokio::test]
    async fn test_get_inventory_offline() {
        let state = offline_state();
        let result = get_inventory(&state, InventoryInput { force: false }).await;
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    // -- get_hotbar -------------------------------------------------------------

    #[tokio::test]
    async fn test_get_hotbar_returns_nine_slots_with_empties() {
        let state = state_with_snapshot();
        // state_with_snapshot: hotbar slots 0 (iron_pickaxe) + 1 (oak_planks),
        // held_item_slot = 3.
        let result = get_hotbar(&state, HotbarInput { force: false })
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        let hotbar = parsed["hotbar"].as_array().unwrap();
        assert_eq!(hotbar.len(), 9, "hotbar must render all 9 slots");
        assert_eq!(hotbar[0]["item_id"], "iron_pickaxe");
        assert_eq!(hotbar[1]["count"], 64);
        assert!(hotbar[2].is_null(), "empty slots render as null");
        assert!(hotbar[8].is_null());
        assert_eq!(parsed["held_item_slot"], 3);
    }

    #[tokio::test]
    async fn test_get_hotbar_offline() {
        let state = offline_state();
        let result = get_hotbar(&state, HotbarInput { force: false }).await;
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    // -- get_bot_status ---------------------------------------------------------

    #[tokio::test]
    async fn test_get_bot_status_online() {
        let state = state_with_snapshot();
        let result = get_bot_status(&state, BotStatusInput { force: false })
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["connected"], true);
        assert_eq!(parsed["bot_busy"], false);
        assert_eq!(parsed["position"], serde_json::json!([0, 64, 0]));
        assert_eq!(parsed["gamemode"], "survival");
        assert_eq!(parsed["health"], 18.0);
        assert_eq!(parsed["snapshot_timestamp"], 42);
        assert_eq!(parsed["held_item_slot"], 3);
    }

    #[tokio::test]
    async fn test_get_bot_status_offline_reports_connected_false() {
        let state = offline_state();
        let result = get_bot_status(&state, BotStatusInput { force: false })
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["connected"], false);
    }

    // -- get_nearby_blocks -----------------------------------------------------

    #[test]
    fn test_get_nearby_blocks_radius_1() {
        let state = state_with_snapshot();
        let result = get_nearby_blocks(&state, 1, None, false, 500).unwrap();
        // Within radius 1 of (0,64,0): stone at (0,64,0), dirt at (0,65,0)
        assert!(result.contains("stone"));
        assert!(result.contains("dirt"));
        // diamond_ore at (10,64,0) is too far
        assert!(!result.contains("diamond_ore"));
    }

    #[test]
    fn test_get_nearby_blocks_filter() {
        let state = state_with_snapshot();
        let result = get_nearby_blocks(&state, 1, Some("stone".into()), false, 500).unwrap();
        assert!(result.contains("stone"));
        assert!(!result.contains("dirt"));
    }

    #[test]
    fn test_get_nearby_blocks_empty_filter_acts_as_none() {
        let state = state_with_snapshot();
        let result = get_nearby_blocks(&state, 1, Some("".into()), false, 500).unwrap();
        assert!(result.contains("stone"));
        assert!(result.contains("dirt"));
    }

    #[test]
    fn test_get_nearby_blocks_offline() {
        let state = offline_state();
        let result = get_nearby_blocks(&state, 5, None, false, 500);
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    #[test]
    fn test_get_nearby_blocks_top_only_keeps_highest_of_column() {
        // Column (0,0) holds stone at y=64 and dirt at y=65; top_only must
        // return only the highest (dirt).
        let state = state_with_snapshot();
        let result = get_nearby_blocks(&state, 1, None, true, 500).unwrap();
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["top_only"], json!(true));
        assert_eq!(v["truncated"], json!(false));
        let blocks = v["blocks"].as_array().unwrap();
        assert_eq!(blocks.len(), 1, "one column, one top block: {result}");
        let b = &blocks[0];
        assert_eq!(b["block_type"], json!("dirt"));
        assert!(b["position"]["y"] == json!(65));
    }

    #[test]
    fn test_get_nearby_blocks_max_blocks_truncates_and_flags() {
        let state = state_with_snapshot();
        let result = get_nearby_blocks(&state, 100, None, false, 1).unwrap();
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["truncated"], json!(true), "got: {result}");
        assert_eq!(v["count"], json!(1));
        assert_eq!(
            v["total_matched"],
            json!(3),
            "all 3 blocks matched before the cap"
        );
    }

    #[test]
    fn test_get_nearby_blocks_invalid_max_blocks_rejected() {
        let state = state_with_snapshot();
        let result = get_nearby_blocks(&state, 5, None, false, 0);
        assert!(
            matches!(result, Err(BotError::InvalidParams(ref msg)) if msg.contains("max_blocks")),
            "expected InvalidParams for max_blocks=0, got: {result:?}"
        );
        let result = get_nearby_blocks(&state, 5, None, false, 10001);
        assert!(
            matches!(result, Err(BotError::InvalidParams(ref msg)) if msg.contains("max_blocks")),
            "expected InvalidParams for max_blocks=10001, got: {result:?}"
        );
    }

    #[test]
    fn test_get_nearby_blocks_response_shape() {
        let state = state_with_snapshot();
        let result = get_nearby_blocks(&state, 1, None, false, 500).unwrap();
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert!(
            v.get("blocks").is_some(),
            "response must be an object with blocks"
        );
        assert!(v.get("count").is_some());
        assert!(v.get("total_matched").is_some());
        assert!(v.get("truncated").is_some());
        assert!(v.get("top_only").is_some());
        assert_eq!(v["truncated"], json!(false));
    }

    // -- get_nearby_entities ---------------------------------------------------

    #[test]
    fn test_get_nearby_entities_radius_1() {
        let state = state_with_snapshot();
        let result = get_nearby_entities(&state, 1).unwrap();
        // L-13 (rewritten from the bare-array assertions): the response is an
        // OBJECT `{entities, count, truncated}` mirroring get_nearby_blocks
        // (R-11), so the assertions must parse the object rather than scan
        // the raw JSON string for a bare array.
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        let entities = v["entities"]
            .as_array()
            .expect("entities must be a JSON array inside the object");
        let joined = entities
            .iter()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join(",");
        assert!(joined.contains("zombie"));
        assert!(!joined.contains("creeper")); // creeper at (100,64,0) is far
        assert_eq!(v["count"], json!(1));
        assert_eq!(v["truncated"], json!(false));
    }

    #[test]
    fn test_get_nearby_entities_large_radius() {
        let state = state_with_snapshot();
        // radius = 100 (the maximum allowed by runtime validation) still
        // catches both nearby and far entities.
        let result = get_nearby_entities(&state, 100).unwrap();
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        let entities = v["entities"]
            .as_array()
            .expect("entities must be a JSON array inside the object");
        let joined = entities
            .iter()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join(",");
        assert!(joined.contains("zombie"));
        assert!(joined.contains("creeper"));
        assert_eq!(v["count"], json!(2));
        assert_eq!(v["truncated"], json!(false));
    }

    #[test]
    fn test_get_nearby_entities_offline() {
        let state = offline_state();
        let result = get_nearby_entities(&state, 10);
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    // -- L-13: object shape with max_entities cap ----------------------------

    #[test]
    fn test_get_nearby_entities_object_shape_with_truncation() {
        let state = state_with_snapshot();
        // radius 100 catches both entities; max_entities=1 caps the payload
        // and must set truncated:true honestly.
        let result = get_nearby_entities_capped(&state, 100, 1).unwrap();
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert!(
            v.get("entities").is_some(),
            "response must be an object with entities, got: {result}"
        );
        assert_eq!(v["count"], json!(1), "count must be capped, got: {result}");
        assert_eq!(v["truncated"], json!(true), "got: {result}");
        assert_eq!(v["entities"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_get_nearby_entities_invalid_max_entities_rejected() {
        let state = state_with_snapshot();
        let result = get_nearby_entities_capped(&state, 5, 0);
        assert!(
            matches!(result, Err(BotError::InvalidParams(ref msg)) if msg.contains("max_entities")),
            "expected InvalidParams for max_entities=0, got: {result:?}"
        );
        let result = get_nearby_entities_capped(&state, 5, 10001);
        assert!(
            matches!(result, Err(BotError::InvalidParams(ref msg)) if msg.contains("max_entities")),
            "expected InvalidParams for max_entities=10001, got: {result:?}"
        );
    }

    // -- L-17: get_nearby_blocks single-entry cache --------------------------

    /// Serializes the two nearby-blocks cache tests: they deliberately plant
    /// entries under the same file-local cache key, so they must not
    /// interleave with each other (other tests use distinct keys and are
    /// unaffected).
    static CACHE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn test_get_nearby_blocks_cache_hit_on_same_seq_and_params() {
        let _guard = CACHE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let state = state_with_snapshot();
        let seq = state.read_snapshot().snapshot_seq;
        // Plant a deliberately WRONG entry under the request's key: if the
        // handler serves it verbatim, the cache is genuinely consulted (a
        // recompute would return the real data). Radius 7 + filter "stone"
        // is a key no other test uses (the stone block at (0,64,0) lies
        // inside radius 7 of the centre).
        *NEARBY_BLOCKS_CACHE
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(NearbyBlocksCache {
            snapshot_seq: seq,
            radius: 7,
            filter_type: Some("stone".into()),
            top_only: false,
            max_blocks: 500,
            response: "CACHED-PLANT".into(),
        });
        let result = get_nearby_blocks(&state, 7, Some("stone".into()), false, 500).unwrap();
        assert_eq!(
            result, "CACHED-PLANT",
            "same snapshot_seq + params must be served from the cache"
        );

        // After clearing the cache the same call recomputes the real data.
        *NEARBY_BLOCKS_CACHE
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
        let fresh = get_nearby_blocks(&state, 7, Some("stone".into()), false, 500).unwrap();
        assert_ne!(fresh, "CACHED-PLANT");
        assert!(fresh.contains("stone"));
    }

    #[test]
    fn test_get_nearby_blocks_cache_invalidates_on_seq_change() {
        let _guard = CACHE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let state = state_with_snapshot();
        let seq = state.read_snapshot().snapshot_seq;
        *NEARBY_BLOCKS_CACHE
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(NearbyBlocksCache {
            snapshot_seq: seq,
            radius: 7,
            filter_type: Some("stone".into()),
            top_only: false,
            max_blocks: 500,
            response: "STALE-PLANT".into(),
        });
        // Re-store the SAME content (identical timestamp): only the
        // monotonic snapshot_seq changes. A cache keyed on timestamp would
        // wrongly serve the stale entry.
        let snap = state.read_snapshot().as_ref().clone();
        state.update_snapshot(snap);
        assert_ne!(
            state.read_snapshot().snapshot_seq,
            seq,
            "update_snapshot must bump snapshot_seq"
        );
        let result = get_nearby_blocks(&state, 7, Some("stone".into()), false, 500).unwrap();
        assert_ne!(
            result, "STALE-PLANT",
            "snapshot_seq change must invalidate the cache"
        );
        assert!(result.contains("stone"));
    }

    // -- L-18: probe envelope timeout ----------------------------------------

    /// A probe whose responder never replies must NOT hang for the 30 s
    /// command timeout: the short envelope (50 ms here) fires CommandTimeout,
    /// and the probe must preserve the previous cached value (Some(true)),
    /// never flip it to Some(false) and never surface an error.
    #[tokio::test]
    async fn test_probe_uses_short_timeout() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        state.set_online(true);
        state.set_commands_probe(Some(true));

        let (sender, mut receiver) = crate::channel::create_command_channel(4, Arc::clone(&state));
        // Responder that accepts the probe but never replies (a busy
        // executor stuck behind a long-running action).
        let hold = tokio::spawn(async move {
            while let Some(wrapped) = receiver.recv().await {
                drop(wrapped); // accepted, never answered
            }
        });

        let start = std::time::Instant::now();
        let probe =
            probe_commands_enabled_with_timeout(&state, &sender, Duration::from_millis(50)).await;
        let elapsed = start.elapsed();

        assert_eq!(
            probe,
            Some(true),
            "timeout must preserve the previous cached value, got: {probe:?}"
        );
        assert_eq!(
            state.get_commands_probe(),
            Some(true),
            "cache must be unchanged"
        );
        assert!(
            elapsed < Duration::from_secs(1),
            "probe must not block for the 30 s command timeout, took {elapsed:?}"
        );
        hold.abort();
    }

    // -- L-24: image_size contract the corrected prose describes -------------

    #[test]
    fn test_get_world_view_annotation_image_size_matches_doc() {
        let state = state_with_snapshot();
        let contents = get_world_view(&state, 8, 4).unwrap();
        match &contents[1].raw {
            rmcp::model::RawContent::Text(t) => {
                let v: serde_json::Value = serde_json::from_str(&t.text).unwrap();
                assert_eq!(
                    v["image_size"],
                    json!(68),
                    "radius=8, scale=4 must be (2*8+1)*4 = 68 px per side (L-24 doc contract)"
                );
            }
            other => panic!("expected Text content, got: {other:?}"),
        }
        let contents = get_world_view(&state, 8, 1).unwrap();
        match &contents[1].raw {
            rmcp::model::RawContent::Text(t) => {
                let v: serde_json::Value = serde_json::from_str(&t.text).unwrap();
                assert_eq!(v["image_size"], json!(17), "scale=1 must be 17 px per side");
            }
            other => panic!("expected Text content, got: {other:?}"),
        }
    }

    // -- get_chunk_summary -----------------------------------------------------

    #[test]
    fn test_get_chunk_summary_online() {
        let state = state_with_snapshot();
        let result = get_chunk_summary(&state).unwrap();
        assert!(result.contains("[0,0]"));
        assert!(result.contains("[-1,0]"));
    }

    #[test]
    fn test_get_chunk_summary_offline() {
        let state = offline_state();
        let result = get_chunk_summary(&state);
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    // -- is_connected ----------------------------------------------------------

    #[test]
    fn test_is_connected_online() {
        let state = state_with_snapshot();
        let result = is_connected(&state).unwrap();
        assert_eq!(result, r#"{"connected":true}"#);
    }

    #[test]
    fn test_is_connected_offline() {
        let state = offline_state();
        let result = is_connected(&state).unwrap();
        assert_eq!(result, r#"{"connected":false}"#);
    }

    // -- get_server_info -------------------------------------------------------

    /// Create a sender backed by a responder that replies with `reply` to
    /// every command (used to drive the `/seed` probe).
    fn make_sender_with_reply(
        reply: Result<BotResult, BotError>,
    ) -> (BotCommandSender, tokio::task::JoinHandle<()>) {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, mut receiver) = crate::channel::create_command_channel(10, state);
        let handle = tokio::spawn(async move {
            while let Some(wrapped) = receiver.recv().await {
                let _ = wrapped.respond_to.send(reply.clone());
            }
        });
        (sender, handle)
    }

    /// A `BotResult` mirroring the executor's success reply for `/seed`.
    fn ok_result() -> BotResult {
        BotResult {
            success: true,
            message: "Executed command: /seed (server: Seed: [12345])".into(),
            data: None,
        }
    }

    #[tokio::test]
    async fn test_get_server_info_probes_and_caches_true() {
        let state = state_with_snapshot();
        let (sender, handle) = make_sender_with_reply(Ok(ok_result()));
        // commands_enabled is None → the probe runs; the server accepts.
        let result = get_server_info(&state, &sender, ServerInfoInput { refresh: false })
            .await
            .expect("probe accepted must not error");
        assert_eq!(
            state.get_commands_probe(),
            Some(true),
            "accepted /seed probe must be cached"
        );
        let parsed: serde_json::Value = serde_json::from_str(&result).expect("valid JSON");
        assert_eq!(parsed["gamemode"], "survival");
        // `bot_busy` is part of the response contract.
        assert!(parsed["bot_busy"].is_boolean());
        handle.abort();
    }

    #[tokio::test]
    async fn test_get_server_info_probes_rejected() {
        let state = state_with_snapshot();
        let (sender, handle) = make_sender_with_reply(Err(BotError::CommandRejected {
            command: "/seed".into(),
            feedback: "You do not have permission to use this command".into(),
        }));
        let result = get_server_info(&state, &sender, ServerInfoInput { refresh: false })
            .await
            .expect("rejected probe must not surface as a tool error");
        assert_eq!(
            state.get_commands_probe(),
            Some(false),
            "rejected /seed probe must be cached as false"
        );
        assert!(result.contains("gamemode"));
        handle.abort();
    }

    #[tokio::test]
    async fn test_get_server_info_cached_result_skips_probe() {
        let state = state_with_snapshot();
        // Pre-cache a probe result — the call must NOT re-probe (no sender
        // command would be issued; a fresh sender with a broken responder
        // that never replies would hang otherwise).
        state.set_commands_probe(Some(true));
        let (sender, handle) = make_sender_with_reply(Ok(ok_result()));
        let result = get_server_info(&state, &sender, ServerInfoInput { refresh: false })
            .await
            .expect("cached probe must not error");
        assert!(result.contains("gamemode"));
        handle.abort();
    }

    #[tokio::test]
    async fn test_get_server_info_refresh_true_reprobes() {
        let state = state_with_snapshot();
        // Cached true, but refresh=true forces a re-probe that the server
        // now rejects → the cache flips to false.
        state.set_commands_probe(Some(true));
        let (sender, handle) = make_sender_with_reply(Err(BotError::CommandRejected {
            command: "/seed".into(),
            feedback: "Unknown command".into(),
        }));
        let _ = get_server_info(&state, &sender, ServerInfoInput { refresh: true }).await;
        assert_eq!(
            state.get_commands_probe(),
            Some(false),
            "refresh=true must re-run the probe"
        );
        handle.abort();
    }

    #[tokio::test]
    async fn test_get_server_info_with_commands_enabled() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        state.set_online(true);
        let snap = WorldSnapshot {
            commands_enabled: Some(true),
            self_player: SelfPlayer {
                gamemode: GameMode::Creative,
                ..Default::default()
            },
            ..Default::default()
        };
        state.update_snapshot(snap);
        // Pre-cache the probe so no sender command is issued.
        state.set_commands_probe(Some(true));
        let (sender, handle) = make_sender_with_reply(Ok(ok_result()));

        let result = get_server_info(&state, &sender, ServerInfoInput { refresh: false })
            .await
            .expect("valid server info");
        let parsed: serde_json::Value = serde_json::from_str(&result).expect("valid JSON");
        assert_eq!(parsed["commands_enabled"], true);
        assert_eq!(parsed["gamemode"], "creative");
        handle.abort();
    }

    #[tokio::test]
    async fn test_get_server_info_offline() {
        let state = offline_state();
        let (sender, handle) = make_sender_with_reply(Ok(ok_result()));
        let result = get_server_info(&state, &sender, ServerInfoInput::default()).await;
        assert!(matches!(result, Err(BotError::Offline(_))));
        handle.abort();
    }

    // -- get_world_view --------------------------------------------------------

    /// Verify that the multi-content response `[image, text]` is returned
    /// for a valid online call.
    #[test]
    fn test_get_world_view_online_returns_image_and_text() {
        let state = state_with_snapshot();
        let contents = get_world_view(&state, 4, 1).unwrap();
        assert_eq!(
            contents.len(),
            2,
            "response should contain [image, text-annotation]"
        );
        // First content: image.
        match &contents[0].raw {
            rmcp::model::RawContent::Image(img) => {
                assert_eq!(img.mime_type, "image/png");
                assert!(!img.data.is_empty(), "base64 data should be non-empty");
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(&img.data)
                    .expect("base64 decode");
                assert!(
                    decoded.starts_with(&[0x89, 0x50, 0x4E, 0x47]),
                    "decoded bytes should start with PNG magic, got: {:?}",
                    &decoded[..4.min(decoded.len())]
                );
            }
            other => panic!("expected Image content, got: {other:?}"),
        }
        // Second content: text annotation (JSON).
        match &contents[1].raw {
            rmcp::model::RawContent::Text(text) => {
                let parsed: serde_json::Value =
                    serde_json::from_str(&text.text).expect("annotation should be valid JSON");
                assert_eq!(parsed["radius"], 4);
                assert_eq!(parsed["scale"], 1);
                assert!(
                    parsed["center"].is_array(),
                    "annotation should include centre"
                );
                assert!(parsed["snapshot_timestamp"].is_u64());
            }
            other => panic!("expected Text content, got: {other:?}"),
        }
    }

    /// The annotation counts must describe the VIEW, not the whole snapshot.
    /// Regression: they used to be `snapshot.blocks.len()` /
    /// `snapshot.entities.len()` — 3 blocks / 2 entities here, but radius=1
    /// only shows 1 block column and 1 entity.
    #[test]
    fn test_get_world_view_annotation_counts_are_view_scoped() {
        let state = state_with_snapshot();
        let contents = get_world_view(&state, 1, 1).unwrap();
        match &contents[1].raw {
            rmcp::model::RawContent::Text(text) => {
                let parsed: serde_json::Value =
                    serde_json::from_str(&text.text).expect("annotation should be valid JSON");
                // Centre (0,64,0), radius 1: (0,64,0)+(0,65,0) share a column
                // (1 column), diamond_ore at x=10 is out of view, zombie at
                // (1,64,0) is in view, creeper at x=100 is out.
                assert_eq!(
                    parsed["block_count"], 1,
                    "annotation block_count must be view-scoped, got: {parsed}"
                );
                assert_eq!(
                    parsed["entity_count"], 1,
                    "annotation entity_count must be view-scoped, got: {parsed}"
                );
            }
            other => panic!("expected Text content, got: {other:?}"),
        }
    }

    /// A cache hit must return the same view-scoped counts as a fresh render
    /// (the counts are part of the cached annotation).
    #[test]
    fn test_get_world_view_cache_hit_preserves_view_counts() {
        let state = state_with_snapshot();
        let first = get_world_view(&state, 1, 1).unwrap();
        let second = get_world_view(&state, 1, 1).unwrap();
        match (&first[1].raw, &second[1].raw) {
            (rmcp::model::RawContent::Text(a), rmcp::model::RawContent::Text(b)) => {
                let ja: serde_json::Value = serde_json::from_str(&a.text).expect("valid JSON");
                let jb: serde_json::Value = serde_json::from_str(&b.text).expect("valid JSON");
                assert_eq!(
                    ja["block_count"], jb["block_count"],
                    "cache hit must preserve block_count"
                );
                assert_eq!(
                    ja["entity_count"], jb["entity_count"],
                    "cache hit must preserve entity_count"
                );
                assert_eq!(ja["block_count"], 1);
            }
            other => panic!("expected two text contents, got: {other:?}"),
        }
    }

    #[test]
    fn test_get_world_view_offline_returns_error() {
        let state = offline_state();
        let result = get_world_view(&state, 4, 1);
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    #[test]
    fn test_get_world_view_invalid_radius_zero() {
        let state = state_with_snapshot();
        let result = get_world_view(&state, 0, 1);
        assert!(matches!(result, Err(BotError::InvalidParams(_))));
    }

    #[test]
    fn test_get_world_view_invalid_radius_too_large() {
        let state = state_with_snapshot();
        let result = get_world_view(&state, 33, 1);
        assert!(matches!(result, Err(BotError::InvalidParams(_))));
    }

    #[test]
    fn test_get_world_view_radius_boundaries_valid() {
        let state = state_with_snapshot();
        // radius = 1 and radius = 32 should both succeed.
        for radius in [1u8, 32] {
            let contents = get_world_view(&state, radius, 1).unwrap();
            assert!(
                matches!(contents[0].raw, rmcp::model::RawContent::Image(_)),
                "radius {radius} should produce image content"
            );
        }
    }

    /// Different scale values should all succeed and produce valid image
    /// content. Scale 8 produces a much larger PNG than scale 1.
    #[test]
    fn test_get_world_view_scale_values() {
        let state = state_with_snapshot();
        for scale in [1u8, 2, 4, 8] {
            let contents = get_world_view(&state, 4, scale).unwrap();
            match &contents[0].raw {
                rmcp::model::RawContent::Image(img) => {
                    let decoded = base64::engine::general_purpose::STANDARD
                        .decode(&img.data)
                        .expect("base64 decode");
                    assert!(
                        decoded.starts_with(&[0x89, 0x50, 0x4E, 0x47]),
                        "scale {scale} should produce a valid PNG"
                    );
                }
                other => panic!("scale {scale}: expected Image content, got: {other:?}"),
            }
        }
    }

    /// Invalid scale values (0, 3, 5, 9) should fall back to scale=1
    /// rather than erroring — the schema advertises 1-8 but the renderer
    /// clamps gracefully to keep clients from breaking.
    #[test]
    fn test_get_world_view_invalid_scale_falls_back_to_1() {
        let state = state_with_snapshot();
        for invalid_scale in [0u8, 3, 5, 9, 100, 255] {
            let contents = get_world_view(&state, 2, invalid_scale).unwrap();
            // Annotation should report scale=1 (the fallback).
            match &contents[1].raw {
                rmcp::model::RawContent::Text(text) => {
                    let parsed: serde_json::Value =
                        serde_json::from_str(&text.text).expect("valid JSON");
                    assert_eq!(
                        parsed["scale"], 1,
                        "invalid scale {invalid_scale} should fall back to 1"
                    );
                }
                other => panic!("expected Text content, got: {other:?}"),
            }
        }
    }

    /// Two consecutive calls with the same snapshot timestamp + radius +
    /// scale should hit the cache — the second call returns the cached
    /// bytes without re-rendering. We verify this by checking that the
    /// annotation JSON contains the same snapshot_timestamp on both calls.
    #[test]
    fn test_get_world_view_cache_hit_on_repeat_call() {
        let state = state_with_snapshot();
        let first = get_world_view(&state, 4, 1).unwrap();
        let second = get_world_view(&state, 4, 1).unwrap();
        // Both should return 2-element [image, text] responses.
        assert_eq!(first.len(), 2);
        assert_eq!(second.len(), 2);
        // Annotation timestamps should match (same snapshot).
        let first_ts = match &first[1].raw {
            rmcp::model::RawContent::Text(t) => {
                let v: serde_json::Value = serde_json::from_str(&t.text).unwrap();
                v["snapshot_timestamp"].as_u64().unwrap()
            }
            _ => panic!("expected text"),
        };
        let second_ts = match &second[1].raw {
            rmcp::model::RawContent::Text(t) => {
                let v: serde_json::Value = serde_json::from_str(&t.text).unwrap();
                v["snapshot_timestamp"].as_u64().unwrap()
            }
            _ => panic!("expected text"),
        };
        assert_eq!(
            first_ts, second_ts,
            "cache hit should return same timestamp"
        );
    }

    /// A new snapshot revision with the SAME seconds-granularity timestamp
    /// must also invalidate the cache. This is the regression test for the
    /// 500 ms-snapshot / 1 s-timestamp collision: two consecutive builds can
    /// share a timestamp, but `snapshot_seq` is monotonic and must be the
    /// cache key.
    #[test]
    fn test_get_world_view_cache_invalidates_on_snapshot_seq_change() {
        let state = state_with_snapshot();
        let first = get_world_view(&state, 4, 1).unwrap();
        let first_ts = match &first[1].raw {
            rmcp::model::RawContent::Text(t) => {
                let v: serde_json::Value = serde_json::from_str(&t.text).unwrap();
                v["snapshot_timestamp"].as_u64().unwrap()
            }
            _ => panic!("expected text"),
        };
        let first_seq = state.get_world_view_cache().unwrap().snapshot_seq;

        // Update the snapshot but keep the SAME timestamp.
        let mut snap = state.read_snapshot().as_ref().clone();
        snap.timestamp = first_ts;
        state.update_snapshot(snap);

        let second = get_world_view(&state, 4, 1).unwrap();
        let second_seq = state.get_world_view_cache().unwrap().snapshot_seq;
        assert_ne!(
            first_seq, second_seq,
            "same-timestamp snapshot update must invalidate the cache"
        );
        assert_eq!(second.len(), 2);
    }

    /// A snapshot update (new timestamp) should invalidate the cache —
    /// the second call should re-render with the new timestamp.
    #[test]
    fn test_get_world_view_cache_invalidates_on_snapshot_update() {
        let state = state_with_snapshot();
        let first = get_world_view(&state, 4, 1).unwrap();
        let first_ts = match &first[1].raw {
            rmcp::model::RawContent::Text(t) => {
                let v: serde_json::Value = serde_json::from_str(&t.text).unwrap();
                v["snapshot_timestamp"].as_u64().unwrap()
            }
            _ => panic!("expected text"),
        };

        // Update the snapshot with a new timestamp.
        let mut snap = state.read_snapshot().as_ref().clone();
        snap.timestamp = first_ts + 1;
        state.update_snapshot(snap);

        let second = get_world_view(&state, 4, 1).unwrap();
        let second_ts = match &second[1].raw {
            rmcp::model::RawContent::Text(t) => {
                let v: serde_json::Value = serde_json::from_str(&t.text).unwrap();
                v["snapshot_timestamp"].as_u64().unwrap()
            }
            _ => panic!("expected text"),
        };
        assert_ne!(
            first_ts, second_ts,
            "snapshot update should invalidate cache"
        );
    }

    /// A change in radius or scale should also invalidate the cache.
    #[test]
    fn test_get_world_view_cache_invalidates_on_radius_change() {
        let state = state_with_snapshot();
        let first = get_world_view(&state, 4, 1).unwrap();
        let second = get_world_view(&state, 8, 1).unwrap();
        let first_r = match &first[1].raw {
            rmcp::model::RawContent::Text(t) => {
                let v: serde_json::Value = serde_json::from_str(&t.text).unwrap();
                v["radius"].as_u64().unwrap()
            }
            _ => panic!("expected text"),
        };
        let second_r = match &second[1].raw {
            rmcp::model::RawContent::Text(t) => {
                let v: serde_json::Value = serde_json::from_str(&t.text).unwrap();
                v["radius"].as_u64().unwrap()
            }
            _ => panic!("expected text"),
        };
        assert_ne!(first_r, second_r, "radius change should re-render");
        assert_eq!(first_r, 4);
        assert_eq!(second_r, 8);
    }

    // ── clamp_to_i32 (P0-#1) ───────────────────────────────────────

    /// `clamp_to_i32` is the saturating cast used in `get_nearby_blocks` /
    /// `get_nearby_entities` to keep `u32` radius values above `i32::MAX`
    /// from silently wrapping to negative (which would make the Chebyshev
    /// filter return nothing).
    #[test]
    fn test_radius_clamp_overflow() {
        // 5_000_000_000 exceeds u32::MAX (4_294_967_295) so we use
        // `u32::MAX` directly to exercise the saturate branch.
        assert_eq!(clamp_to_i32(u32::MAX), i32::MAX);
        assert_eq!(clamp_to_i32(4_000_000_000_u32), i32::MAX);
        assert_eq!(clamp_to_i32(100_u32), 100);
        assert_eq!(clamp_to_i32(i32::MAX as u32), i32::MAX);
    }

    /// End-to-end: an oversized `u32` radius is now rejected at runtime
    /// (not silently clamped). The `#[schemars(range(min = 1, max = 100))]`
    /// JSON Schema annotation is now enforced.
    #[test]
    fn test_get_nearby_blocks_oversized_radius_does_not_panic() {
        let state = state_with_snapshot();
        // u32::MAX is rejected by runtime bounds check instead of being
        // silently clamped (and potentially wrapping via `as i32`).
        let result = get_nearby_blocks(&state, u32::MAX, None, false, 500);
        assert!(
            result.is_err(),
            "u32::MAX must be rejected by radius validation, got: {result:?}"
        );
        // Also verify the error is InvalidParams, not a panic.
        assert!(matches!(result, Err(BotError::InvalidParams(_))));
    }

    /// Same regression for `get_nearby_entities`.
    #[test]
    fn test_get_nearby_entities_oversized_radius_does_not_panic() {
        let state = state_with_snapshot();
        let result = get_nearby_entities(&state, u32::MAX);
        assert!(
            result.is_err(),
            "u32::MAX must be rejected by radius validation, got: {result:?}"
        );
        assert!(matches!(result, Err(BotError::InvalidParams(_))));
    }
}
