//! MCP server setup, transport, and request dispatch.
//!
//! Uses rmcp 1.8.0 with `#[tool_router]`/`#[tool_handler]` macros to define
//! 41 MCP tools. All logging goes to stderr via `tracing`.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{
    Router,
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    middleware::{Next, from_fn_with_state},
    response::{IntoResponse, Response},
};
use rmcp::{
    ErrorData, ServerHandler, ServiceExt,
    handler::server::{tool::ToolCallContext, wrapper::Parameters},
    model::{
        CallToolRequestParams, CallToolResult, Implementation, InitializeRequestParams,
        InitializeResult, ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerInfo,
    },
    tool, tool_handler, tool_router,
    transport::io::stdio,
    transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    },
};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use crate::channel::{BotCommandSender, ReceiverSlot};
use crate::error::BotError;
use crate::mcp::tools_act::ActInput;
use crate::mcp::tools_block::{BreakBlockInput, PlaceBlockInput, UseItemOnBlockInput};
use crate::mcp::tools_chat::{ExecuteCommandInput, SendChatInput, SetGameModeInput};
use crate::mcp::tools_combat::{AttackEntityInput, ShieldBlockInput};
use crate::mcp::tools_container::{
    CloseContainerInput, OpenContainerInput, PutIntoContainerInput, TakeFromContainerInput,
};
use crate::mcp::tools_item::{
    CollectItemsInput, DropItemInput, EquipToolInput, GiveItemInput, SetHotbarItemInput,
    SwitchHotbarSlotInput, UseItemInput,
};
use crate::mcp::tools_movement::{
    FlyToInput, JumpInput, MoveToInput, SmartMoveInput, TeleportInput, WalkDirectionInput,
};
use crate::mcp::tools_query::{
    BotStatusInput, GetWorldViewInput, HotbarInput, InventoryInput, NearbyBlocksInput,
    NearbyEntitiesInput, SelfInfoInput, ServerInfoInput,
};
use crate::mcp::tools_settings::UpdateSettingsInput;
use crate::state::{McpServerStatus, SharedState};

// ---------------------------------------------------------------------------
// McpBotServer
// ---------------------------------------------------------------------------

/// MCP server struct holding shared state, the bot command channel, and the
/// command-receiver slot.
///
/// The `Arc<SharedState>` is read directly by query tools; action tools
/// send [`BotCommand`](crate::types::BotCommand) through the sender.
///
/// The `receiver` slot is needed by the `connect_bot` tool: spawning a bot
/// connection hands the slot to the spawn helper so the fresh executor can
/// lease the receiver (the same slot the UI path uses).
pub struct McpBotServer {
    state: Arc<SharedState>,
    sender: BotCommandSender,
    receiver: ReceiverSlot,
}

impl McpBotServer {
    /// Create a new MCP server instance.
    pub fn new(state: Arc<SharedState>, sender: BotCommandSender, receiver: ReceiverSlot) -> Self {
        Self {
            state,
            sender,
            receiver,
        }
    }
}

// ---------------------------------------------------------------------------
// Tool Router — 41 MCP tool registrations
// ---------------------------------------------------------------------------

#[tool_router]
impl McpBotServer {
    // ── Query tools (read_only) ──────────────────────────────

    #[tool(
        description = "Get information about the bot's own player. force=true (default) triggers an immediate snapshot refresh so the result reflects the latest world state.",
        annotations(read_only_hint = true)
    )]
    async fn get_self_info(
        &self,
        Parameters(input): Parameters<SelfInfoInput>,
    ) -> Result<String, BotError> {
        crate::mcp::tools_query::get_self_info(&self.state, input).await
    }

    #[tool(
        description = "Get the bot's inventory contents. force=true (default) triggers an immediate snapshot refresh so the result reflects the latest container-content packets.",
        annotations(read_only_hint = true)
    )]
    async fn get_inventory(
        &self,
        Parameters(input): Parameters<InventoryInput>,
    ) -> Result<String, BotError> {
        crate::mcp::tools_query::get_inventory(&self.state, input).await
    }

    #[tool(
        description = "Get the bot's 9 hotbar slots (0-8). Occupied slots carry slot/item_id/count; empty slots are null. Also returns the currently selected held_item_slot.",
        annotations(read_only_hint = true)
    )]
    async fn get_hotbar(
        &self,
        Parameters(input): Parameters<HotbarInput>,
    ) -> Result<String, BotError> {
        crate::mcp::tools_query::get_hotbar(&self.state, input).await
    }

    #[tool(
        description = "Lightweight status poll for long-running operations (fly_to / mining / collect_items): connected, bot_busy, position (block + precise), yaw, health, hunger, gamemode, snapshot age. Reads the cached snapshot by default (no forced refresh) and reports connected:false while offline instead of erroring.",
        annotations(read_only_hint = true)
    )]
    async fn get_bot_status(
        &self,
        Parameters(input): Parameters<BotStatusInput>,
    ) -> Result<String, BotError> {
        crate::mcp::tools_query::get_bot_status(&self.state, input).await
    }

    #[tool(
        description = "Get blocks near the bot's position. Optional filter_type does a case-insensitive substring match on block_type. Pass top_only=true to get just the highest block of each column (drastically smaller responses — recommended), and max_blocks caps the result (default 500, truncated flag reports when the cap is hit).",
        annotations(read_only_hint = true)
    )]
    async fn get_nearby_blocks(
        &self,
        Parameters(input): Parameters<NearbyBlocksInput>,
    ) -> Result<String, BotError> {
        crate::mcp::tools_query::get_nearby_blocks(
            &self.state,
            input.radius,
            input.filter_type,
            input.top_only,
            input.max_blocks,
        )
    }

    #[tool(
        description = "Get entities near the bot's position",
        annotations(read_only_hint = true)
    )]
    async fn get_nearby_entities(
        &self,
        Parameters(input): Parameters<NearbyEntitiesInput>,
    ) -> Result<String, BotError> {
        crate::mcp::tools_query::get_nearby_entities_capped(
            &self.state,
            input.radius,
            input.max_entities,
        )
    }

    #[tool(
        description = "Get a summary of loaded chunks",
        annotations(read_only_hint = true)
    )]
    async fn get_chunk_summary(&self) -> Result<String, BotError> {
        crate::mcp::tools_query::get_chunk_summary(&self.state)
    }

    #[tool(
        description = "Check if the bot is connected to a Minecraft server",
        annotations(read_only_hint = true)
    )]
    async fn is_connected(&self) -> Result<String, BotError> {
        crate::mcp::tools_query::is_connected(&self.state)
    }

    #[tool(
        description = "Returns recent chat messages (up to 50). Each message has sender and message fields.",
        annotations(read_only_hint = true)
    )]
    async fn get_chat_history(&self) -> Result<String, BotError> {
        crate::mcp::tools_chat::get_chat_history(&self.state)
    }

    #[tool(
        description = "Reports whether commands are enabled on the server and the current gamemode. commands_enabled is true/false/null, probed live via /seed (cached until refresh=true). Also reports bot_busy.",
        annotations(read_only_hint = true)
    )]
    async fn get_server_info(
        &self,
        Parameters(input): Parameters<ServerInfoInput>,
    ) -> Result<String, BotError> {
        crate::mcp::tools_query::get_server_info(&self.state, &self.sender, input).await
    }

    #[tool(
        description = "Renders a top-down PNG image of nearby blocks and entities for multimodal models. Returns [image, text-annotation] contents: the image is a base64 PNG, the text is a JSON object with centre coords, radius, scale, yaw, and timestamp. Radius 1-32, scale 1/2/4/8 (pixels per block).",
        annotations(read_only_hint = true)
    )]
    async fn get_world_view(
        &self,
        Parameters(input): Parameters<GetWorldViewInput>,
    ) -> Result<Vec<rmcp::model::Content>, BotError> {
        crate::mcp::tools_query::get_world_view(&self.state, input.radius, input.scale)
    }

    // ── Settings & lifecycle tools ──────────────────────────

    #[tool(
        description = "Get the current configuration (server address, bot parameters, MCP transport) plus runtime status. The MCP token is redacted.",
        annotations(read_only_hint = true)
    )]
    async fn get_settings(&self) -> Result<String, BotError> {
        crate::mcp::tools_settings::get_settings(&self.state)
    }

    #[tool(
        description = "Update configuration fields (partial update — only provided fields change). Applied in memory for the running process only; persist settings via MINECRAFT_MCP_* environment variables. Changing mc_address/mc_port/ai_username triggers a bot reconnect when connected. Changes to mcp_transport/mcp_address/mcp_port take effect on process restart.",
        annotations(destructive_hint = true)
    )]
    async fn update_settings(
        &self,
        Parameters(input): Parameters<UpdateSettingsInput>,
    ) -> Result<String, BotError> {
        crate::mcp::tools_settings::update_settings(&self.state, input)
    }

    #[tool(
        description = "Start the bot connection to the configured Minecraft server. No-op if already connected or connecting."
    )]
    async fn connect_bot(&self) -> Result<String, BotError> {
        crate::mcp::tools_settings::connect_bot(&self.state, &self.receiver, &self.sender)
    }

    #[tool(description = "Request the bot to disconnect and stop reconnecting.")]
    async fn disconnect_bot(&self) -> Result<String, BotError> {
        crate::mcp::tools_settings::disconnect_bot(&self.state)
    }

    // ── Movement tools ───────────────────────────────────────

    #[tool(description = "Move the bot to a specific position")]
    async fn move_to(
        &self,
        Parameters(input): Parameters<MoveToInput>,
    ) -> Result<String, BotError> {
        crate::mcp::tools_movement::handle_move_to(&self.state, &self.sender, input).await
    }

    #[tool(
        description = "Walk the bot a number of blocks in a horizontal direction (north/south/east/west or a diagonal). Returns the requested distance and the bot's end position."
    )]
    async fn walk_direction(
        &self,
        Parameters(input): Parameters<WalkDirectionInput>,
    ) -> Result<String, BotError> {
        crate::mcp::tools_movement::handle_walk_direction(&self.state, &self.sender, input).await
    }

    #[tool(description = "Make the bot jump")]
    async fn jump(&self, Parameters(input): Parameters<JumpInput>) -> Result<String, BotError> {
        crate::mcp::tools_movement::handle_jump(&self.state, &self.sender, input).await
    }

    #[tool(
        description = "Teleport the bot to a position via /tp (requires Creative mode and operator/command access)"
    )]
    async fn teleport(
        &self,
        Parameters(input): Parameters<TeleportInput>,
    ) -> Result<String, BotError> {
        crate::mcp::tools_movement::handle_teleport(&self.state, &self.sender, input).await
    }

    #[tool(
        description = "Smart movement toward target with auto-jump over 1-block obstacles. Stops on impassable obstacle and reports it.",
        annotations(destructive_hint = true)
    )]
    async fn smart_move(
        &self,
        Parameters(input): Parameters<SmartMoveInput>,
    ) -> Result<String, BotError> {
        crate::mcp::tools_movement::handle_smart_move(&self.state, &self.sender, input).await
    }

    #[tool(
        description = "Creative mode only. Flies toward target in 3D. Stops on obstacle. Returns reached status.",
        annotations(destructive_hint = true)
    )]
    async fn fly_to(&self, Parameters(input): Parameters<FlyToInput>) -> Result<String, BotError> {
        crate::mcp::tools_movement::handle_fly_to(&self.state, &self.sender, input).await
    }

    // ── Block tools (destructive) ────────────────────────────

    #[tool(
        description = "Break a block at the given position. By default runs the full compound mine flow: approaches the block, picks the best tool (errors with a clear reason when the right tool is missing, e.g. a shovel for grass), mines, and verifies the break. Returns the action result plus the bot's final position. Set use_best_tool=false for the raw single-packet break. In Creative mode, prefer `execute_command` with `/fill` or `/setblock` for bulk building.",
        annotations(destructive_hint = true)
    )]
    async fn break_block(
        &self,
        Parameters(input): Parameters<BreakBlockInput>,
    ) -> Result<String, BotError> {
        crate::mcp::tools_block::handle_break_block(&self.state, &self.sender, input).await
    }

    #[tool(
        description = "Place a block at the given position; the placed block occupies exactly (x, y, z). Placement is verified against the world after the click — success is reported only when the block was observed at the target. y must be in -63..=320; y=-64 is rejected because the clicked block would be at y=-65, outside the world. In Creative mode, prefer `execute_command` with `/fill` or `/setblock` for bulk building.",
        annotations(destructive_hint = true)
    )]
    async fn place_block(
        &self,
        Parameters(input): Parameters<PlaceBlockInput>,
    ) -> Result<String, BotError> {
        crate::mcp::tools_block::handle_place_block(&self.state, &self.sender, input).await
    }

    #[tool(
        description = "Use the held item on a block. Always pass item_slot (0-8) so the correct item is held — e.g. the hotbar slot holding water_bucket when pouring water. The optional face (up/down/north/south/east/west, default up) picks the cell the placement lands in: face up pours water into the cell ABOVE the target block. Placement items (buckets, blocks) are verified against the world after the click — a rejected interaction returns an explicit failure instead of a fake success.",
        annotations(destructive_hint = true)
    )]
    async fn use_item_on_block(
        &self,
        Parameters(input): Parameters<UseItemOnBlockInput>,
    ) -> Result<String, BotError> {
        crate::mcp::tools_block::handle_use_item_on_block(&self.state, &self.sender, input).await
    }

    // ── Item tools (destructive) ─────────────────────────────

    #[tool(
        description = "Switch to a hotbar slot (0-8).",
        annotations(destructive_hint = true)
    )]
    async fn switch_hotbar_slot(
        &self,
        Parameters(input): Parameters<SwitchHotbarSlotInput>,
    ) -> Result<String, BotError> {
        crate::mcp::tools_item::handle_switch_hotbar_slot(&self.state, &self.sender, input).await
    }

    #[tool(
        description = "Drop items from an inventory slot.",
        annotations(destructive_hint = true)
    )]
    async fn drop_item(
        &self,
        Parameters(input): Parameters<DropItemInput>,
    ) -> Result<String, BotError> {
        crate::mcp::tools_item::handle_drop_item(&self.state, &self.sender, input).await
    }

    #[tool(
        description = "Use the currently held item.",
        annotations(destructive_hint = true)
    )]
    async fn use_item(
        &self,
        Parameters(input): Parameters<UseItemInput>,
    ) -> Result<String, BotError> {
        crate::mcp::tools_item::handle_use_item(&self.state, &self.sender, input).await
    }

    #[tool(
        description = "Move an existing inventory stack into a hotbar slot via a swap-click (reliable alternative to /item replace). Requires the item to already be in the inventory.",
        annotations(destructive_hint = true)
    )]
    async fn set_hotbar_item(
        &self,
        Parameters(input): Parameters<SetHotbarItemInput>,
    ) -> Result<String, BotError> {
        crate::mcp::tools_item::handle_set_hotbar_item(&self.state, &self.sender, input).await
    }

    #[tool(
        description = "Give the bot an item via server commands (requires OP). Runs /give <bot> <item> <count>; target=hotbar additionally runs /item replace (with a swap-click fallback if /item replace is rejected). Rejected commands, such as an unknown item id, return a command_rejected error instead of a fake success.",
        annotations(destructive_hint = true)
    )]
    async fn give_item(
        &self,
        Parameters(input): Parameters<GiveItemInput>,
    ) -> Result<String, BotError> {
        crate::mcp::tools_item::handle_give_item(&self.state, &self.sender, input).await
    }

    #[tool(
        description = "Equip the best available tool of a given type.",
        annotations(destructive_hint = true)
    )]
    async fn equip_tool(
        &self,
        Parameters(input): Parameters<EquipToolInput>,
    ) -> Result<String, BotError> {
        crate::mcp::tools_item::handle_equip_tool(&self.state, &self.sender, input).await
    }

    #[tool(
        description = "Walks toward and picks up nearby dropped item entities within radius. Returns count collected.",
        annotations(destructive_hint = true)
    )]
    async fn collect_items(
        &self,
        Parameters(input): Parameters<CollectItemsInput>,
    ) -> Result<String, BotError> {
        crate::mcp::tools_item::handle_collect_items(&self.state, &self.sender, input).await
    }

    // ── Container tools (destructive) ────────────────────────

    #[tool(
        description = "Open a container at the given position",
        annotations(destructive_hint = true)
    )]
    async fn open_container(
        &self,
        Parameters(input): Parameters<OpenContainerInput>,
    ) -> Result<String, BotError> {
        crate::mcp::tools_container::handle_open_container(&self.state, &self.sender, input).await
    }

    #[tool(
        description = "Take items from an open container slot",
        annotations(destructive_hint = true)
    )]
    async fn take_from_container(
        &self,
        Parameters(input): Parameters<TakeFromContainerInput>,
    ) -> Result<String, BotError> {
        crate::mcp::tools_container::handle_take_from_container(&self.state, &self.sender, input)
            .await
    }

    #[tool(
        description = "Put items into an open container slot",
        annotations(destructive_hint = true)
    )]
    async fn put_into_container(
        &self,
        Parameters(input): Parameters<PutIntoContainerInput>,
    ) -> Result<String, BotError> {
        crate::mcp::tools_container::handle_put_into_container(&self.state, &self.sender, input)
            .await
    }

    #[tool(
        description = "Close the currently open container",
        annotations(destructive_hint = true)
    )]
    async fn close_container(
        &self,
        Parameters(input): Parameters<CloseContainerInput>,
    ) -> Result<String, BotError> {
        crate::mcp::tools_container::handle_close_container(&self.state, &self.sender, input).await
    }

    // ── Combat / Chat tools (destructive) ────────────────────

    #[tool(
        description = "Attack an entity by its Minecraft entity ID",
        annotations(destructive_hint = true)
    )]
    async fn attack_entity(
        &self,
        Parameters(input): Parameters<AttackEntityInput>,
    ) -> Result<String, BotError> {
        crate::mcp::tools_combat::handle_attack_entity(&self.state, &self.sender, input).await
    }

    #[tool(
        description = "Hold up shield to block incoming attacks",
        annotations(destructive_hint = true)
    )]
    async fn shield_block(
        &self,
        Parameters(input): Parameters<ShieldBlockInput>,
    ) -> Result<String, BotError> {
        crate::mcp::tools_combat::handle_shield_block(&self.state, &self.sender, input).await
    }

    #[tool(
        description = "Send a chat message to the server",
        annotations(destructive_hint = true)
    )]
    async fn send_chat(
        &self,
        Parameters(SendChatInput { message }): Parameters<SendChatInput>,
    ) -> Result<String, BotError> {
        crate::mcp::tools_chat::handle_send_chat(&self.state, &self.sender, message).await
    }

    #[tool(
        description = "Execute a Minecraft command (requires op). The / prefix is auto-added if omitted.",
        annotations(destructive_hint = true)
    )]
    async fn execute_command(
        &self,
        Parameters(ExecuteCommandInput { command }): Parameters<ExecuteCommandInput>,
    ) -> Result<String, BotError> {
        crate::mcp::tools_chat::handle_execute_command(&self.state, &self.sender, command).await
    }

    #[tool(
        description = "Set the bot's game mode (requires OP permissions). Valid modes: survival, creative, adventure, spectator.",
        annotations(destructive_hint = true)
    )]
    async fn set_game_mode(
        &self,
        Parameters(SetGameModeInput { mode }): Parameters<SetGameModeInput>,
    ) -> Result<String, BotError> {
        crate::mcp::tools_chat::handle_set_game_mode(&self.state, &self.sender, mode).await
    }

    // ── Unified action tool ──────────────────────────────────

    #[tool(
        description = "Unified action tool. Executes one action and returns the result plus nearby blocks, entities, and self info for iterative mining/exploration loops. perception_radius (0-32, default = configured block_perception_radius) bounds the nearby blocks/entities payload — pass a small value (e.g. 2) or 0 to keep responses compact.",
        annotations(destructive_hint = true)
    )]
    async fn act(&self, Parameters(input): Parameters<ActInput>) -> Result<String, BotError> {
        crate::mcp::tools_act::handle_act(&self.state, &self.sender, input).await
    }
}

// ---------------------------------------------------------------------------
// ServerHandler — auto-generated call_tool / list_tools / get_info
// ---------------------------------------------------------------------------

#[tool_handler]
impl ServerHandler for McpBotServer {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.server_info = Implementation::new("minecraft-mcp-rs", env!("CARGO_PKG_VERSION"));
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.instructions = Some(
            "Minecraft bot control via MCP. Use query tools to inspect world state, \
             action tools to control the bot. All destructive operations are annotated. \
             Supported Minecraft version: Java Edition 1.21.11 (the only version \
             supported by the azalea 0.15.1 bot library)."
                .into(),
        );
        info
    }

    // The four request entry points below stamp MCP-request activity so the
    // headless idle watchdog keys on "client is alive and talking to us"
    // rather than "client dispatched a bot command". ZCode spawns per-session
    // probe connections that send initialize/list_tools but never a bot
    // command — keying the watchdog on command activity killed those
    // sessions after 600 s ("MCP server connection closed unexpectedly").
    // The `#[tool_handler]` macro skips generating a method when the impl
    // already defines it (`has_method` guard), so these overrides delegate to
    // the same logic the macro would have generated.

    async fn initialize(
        &self,
        request: InitializeRequestParams,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<InitializeResult, ErrorData> {
        self.state.mark_mcp_activity();
        context.peer.set_peer_info(request);
        Ok(self.get_info())
    }

    async fn ping(
        &self,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<(), ErrorData> {
        self.state.mark_mcp_activity();
        Ok(())
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        self.state.mark_mcp_activity();
        let tcc = ToolCallContext::new(self, request, context);
        Self::tool_router().call(tcc).await
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        self.state.mark_mcp_activity();
        Ok(ListToolsResult {
            tools: Self::tool_router().list_all(),
            meta: None,
            next_cursor: None,
        })
    }
}

// ---------------------------------------------------------------------------
// Server entry point
// ---------------------------------------------------------------------------

/// Start the MCP server on stdio transport.
///
/// This function blocks until the transport is closed, the shutdown token
/// fires, an OS Ctrl+C is received, or — headless only — the idle watchdog
/// fires after `HEADLESS_IDLE_TIMEOUT` without any MCP request activity.
/// It does NOT simply wait for stdin EOF: on Windows a client host may hold
/// both pipe ends open so EOF never arrives, hence the raced exit paths.
/// All logging goes to stderr; stdout is reserved for MCP JSON-RPC
/// messages.
///
/// The `receiver` slot is handed to the server so the `connect_bot` tool can
/// spawn a bot connection using the same receiver the UI path uses.
///
/// `headless` enables the idle watchdog: a headless `--stdio` session whose
/// MCP client has gone away (stdin EOF never arrives on Windows, and the
/// client may hold both pipe ends open) would otherwise linger forever.
/// With the watchdog the process exits once no MCP request has been received
/// for `HEADLESS_IDLE_TIMEOUT` — see `is_headless_idle`.
pub async fn serve_stdio(
    state: Arc<SharedState>,
    sender: BotCommandSender,
    receiver: ReceiverSlot,
    headless: bool,
) {
    // Capture the shutdown token before `state` is moved into the server.
    let shutdown_token = state.shutdown_token();
    // Keep a clone for status updates after `state` is moved into the server.
    let state_for_status = Arc::clone(&state);
    state_for_status.set_mcp_server_status(McpServerStatus::Stdio);
    // Anchor for the idle watchdog: a fresh process gets a full grace period
    // even if the client never dispatches a request. Monotonic (L-23) — a
    // wall-clock NTP jump cannot shorten or lengthen the measured idle span.
    let started_at = Instant::now();
    let server = McpBotServer::new(state, sender, receiver);
    let (stdin, stdout) = stdio();

    info!("MCP server starting on stdio");

    match server.serve((stdin, stdout)).await {
        Ok(running) => {
            info!("MCP server initialized, waiting for transport to close or shutdown");
            // Race the transport-close future against the shutdown token,
            // an OS Ctrl+C (headless only), and the idle watchdog — so the
            // process exits cleanly instead of hanging on stdin EOF, which
            // on Windows may never arrive (inherited console handles have no
            // EOF; a pipe EOF needs every write end closed, and the client
            // host keeps them open).
            //
            // The Ctrl+C arm is headless-gated like the watchdog: registering
            // the handler replaces the OS default process-wide, so in UI mode
            // it would silently stop just the MCP transport while the egui
            // window lived on as a zombie (2026-08-26 review). With no
            // listener, terminal Ctrl+C in UI mode falls back to the OS
            // default — terminating the whole process, which is what a
            // terminal interrupt means.
            tokio::select! {
                _ = running.waiting() => {
                    info!("MCP server transport closed cleanly");
                }
                _ = shutdown_token.cancelled() => {
                    info!("MCP server shutting down (shutdown token triggered)");
                }
                _ = tokio::signal::ctrl_c(), if headless => {
                    info!("Ctrl+C received — shutting down MCP stdio server");
                }
                _ = headless_idle_watchdog(Arc::clone(&state_for_status), started_at), if headless => {
                    info!("headless idle watchdog fired — no MCP request for {:?}, shutting down", HEADLESS_IDLE_TIMEOUT);
                }
            }
        }
        Err(e) => {
            let msg = format!("MCP stdio server failed: {e}");
            error!(error = %e, "MCP server failed");
            state_for_status.set_mcp_server_status(McpServerStatus::Failed(msg.clone()));
            state_for_status.set_last_error(msg);
            // Report the FAILURE, not a later "Stopped": without the return
            // the unconditional Stopped write below overwrote the Failed
            // status the moment it ran, and the UI only ever showed a
            // generic "stopped" (the HTTP path never had this problem).
            return;
        }
    }

    state_for_status.set_mcp_server_status(McpServerStatus::Stopped);
}

/// How long a headless `--stdio` MCP session may go without any MCP request
/// before the process shuts itself down.
///
/// Covers the lingering-process failure seen on Windows: a client host that
/// spawns one process per session and abandons the session without closing
/// the pipe leaves the process alive forever, because stdin EOF never
/// arrives and stdout writes may never fail. Ten minutes with zero MCP
/// request activity is a strong signal the session is abandoned.
///
/// The activity probe is MCP-request activity ([`SharedState::mcp_activity_at`]),
/// NOT bot-command activity: ZCode spawns per-session probe connections
/// that send initialize/list_tools but never dispatch a bot command.
/// Keying the watchdog on command activity killed those healthy sessions
/// after 600 s ("MCP server connection closed unexpectedly").
const HEADLESS_IDLE_TIMEOUT: Duration = Duration::from_secs(600);

/// Poll period for the headless idle watchdog.
const HEADLESS_IDLE_POLL: Duration = Duration::from_secs(5);

/// Pure headless-idle decision, unit-testable without a running server.
///
/// Idle means BOTH the session has been alive for at least `timeout` (a
/// fresh process gets the full grace period before its first request) AND
/// no MCP request has been received within `timeout`.
///
/// All three instants are monotonic (L-23): elapsed time is measured with
/// `saturating_duration_since`, never with wall-clock epoch arithmetic, so
/// an NTP jump can neither fire the watchdog early (forward jump) nor keep
/// the session alive forever (backward jump saturating `now - last` to 0).
fn is_headless_idle(
    now: Instant,
    last_activity: Option<Instant>,
    started_at: Instant,
    timeout: Duration,
) -> bool {
    // `None` (no request ever) means the grace-period check above governs —
    // the old epoch-ms code treated the never-stamped 0 as "idle" too
    // (epoch now minus 0 is always >= timeout).
    let activity_idle = last_activity
        .map(|t| now.saturating_duration_since(t) >= timeout)
        .unwrap_or(true);
    now.saturating_duration_since(started_at) >= timeout && activity_idle
}

/// Resolve once the headless stdio session has been idle (no MCP request
/// received) for [`HEADLESS_IDLE_TIMEOUT`]. Polls [`SharedState`] at a
/// coarse cadence; request activity is recorded by
/// [`SharedState::mark_mcp_activity`] at the entry of every
/// `ServerHandler` request method.
async fn headless_idle_watchdog(state: Arc<SharedState>, started_at: Instant) {
    loop {
        tokio::time::sleep(HEADLESS_IDLE_POLL).await;
        if is_headless_idle(
            Instant::now(),
            state.mcp_activity_at(),
            started_at,
            HEADLESS_IDLE_TIMEOUT,
        ) {
            return;
        }
    }
}

// ---------------------------------------------------------------------------
// HTTP transport
// ---------------------------------------------------------------------------

/// Extract the Bearer token from an `Authorization` header value.
///
/// Returns `Some(token)` only when the header starts with `Bearer ` (case-sensitive
/// per RFC 6750). The returned token is trimmed of surrounding whitespace.
fn extract_bearer_token(header: &str) -> Option<&str> {
    header.strip_prefix("Bearer ").map(str::trim)
}

/// Compare two strings for equality without an early exit on mismatch.
///
/// A plain `a == b` on `str` short-circuits at the first differing byte, so
/// the elapsed time reveals how many leading bytes matched — a timing side
/// channel. Because the UI permits binding the MCP HTTP server to
/// non-loopback addresses (e.g. `0.0.0.0`), the Bearer token can cross an
/// untrusted network, and an attacker able to measure response times could
/// otherwise recover the token one byte at a time.
///
/// Semantics:
/// - Differing lengths return `false` immediately. This leaks the token's
///   *length* through timing, which is acceptable: length alone does not
///   let an attacker brute-force the token contents byte by byte.
/// - Equal lengths XOR every byte pair and OR the differences into an
///   accumulator that touches all bytes regardless of where a mismatch
///   occurs; the strings are equal iff the accumulator is zero.
fn constant_time_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

/// Check whether the request's `Authorization` header carries the expected
/// Bearer token.
///
/// Returns false if:
/// - No Authorization header is present
/// - The header doesn't start with "Bearer "
/// - The extracted token is empty (when expected_token is non-empty)
/// - The token doesn't match expected_token (compared via
///   [`constant_time_eq`] to avoid a timing side channel)
///
/// Reads the expected token from `state.config().mcp_token` on every request so
/// configuration changes take effect immediately.
fn is_bearer_authorized(headers: &HeaderMap, expected_token: &str) -> bool {
    if expected_token.is_empty() {
        return true;
    }
    headers
        .get("Authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(extract_bearer_token)
        .is_some_and(|token| !token.is_empty() && constant_time_eq(token, expected_token))
}

/// Decide whether an HTTP request passes Bearer-token authentication.
///
/// Pure, side-effect-free decision used by [`bearer_auth_middleware`] so the
/// auth policy is testable without a router. Three ordered rules:
///
/// 1. **Auth off** (`auth_enabled == false`) → `true` immediately. This
///    short-circuits BEFORE any token comparison, so no timing side channel
///    is reachable when authentication is disabled.
/// 2. **Empty configured token** → `false`. An empty `expected_token` with
///    auth enabled is a misconfiguration (fail-closed, never open) — the
///    config layer's `validate()` forbids this combination.
/// 3. Otherwise delegate to [`is_bearer_authorized`], which compares via
///    [`constant_time_eq`] to avoid a timing side channel.
fn is_request_authorized(auth_enabled: bool, expected_token: &str, headers: &HeaderMap) -> bool {
    if !auth_enabled {
        return true;
    }
    if expected_token.is_empty() {
        return false;
    }
    is_bearer_authorized(headers, expected_token)
}

/// Axum middleware that enforces the configured Bearer token.
///
/// Returns 401 Unauthorized for missing, empty, or mismatched tokens when
/// authentication is enabled (`mcp_auth_enabled`). If authentication is
/// disabled, all requests pass through. Valid requests are forwarded to the
/// inner MCP streamable HTTP handler.
async fn bearer_auth_middleware(
    State(state): State<Arc<SharedState>>,
    request: Request,
    next: Next,
) -> Response {
    // Scope the config read guard tightly: `RwLockReadGuard` is `!Send`, so
    // keeping it alive across `next.run(request).await` would make the
    // middleware future non-`Send` and break axum's `FromFn` Service impl.
    // The guard is dropped here, before the request is moved into `next.run`.
    let authorized = {
        let cfg = state.read_config();
        is_request_authorized(cfg.mcp_auth_enabled, &cfg.mcp_token, request.headers())
    };
    if authorized {
        next.run(request).await
    } else {
        (StatusCode::UNAUTHORIZED, "Unauthorized").into_response()
    }
}

/// Wait for either the shutdown token or — in headless mode only — an OS
/// Ctrl+C signal.
///
/// `serve_http`'s graceful shutdown races the shutdown token (triggered by
/// window close, `disconnect_bot`, or the headless stdio client exiting)
/// against an OS Ctrl+C, so the process can be terminated cleanly from a
/// terminal in headless mode where nothing else triggers the token.
///
/// `headless` gates the Ctrl+C arm exactly like `serve_stdio`'s: registering
/// the handler replaces the OS default process-wide, so in UI mode it would
/// silently stop just the MCP transport while the window lived on as a
/// zombie (2026-08-26 review). Returning from this future makes axum drain
/// in-flight requests and exit `serve_http`; `main.rs` then triggers the
/// shutdown token (headless) or the window's `Drop` handles it (UI mode).
async fn shutdown_signal(token: CancellationToken, headless: bool) {
    tokio::select! {
        _ = token.cancelled() => {}
        _ = tokio::signal::ctrl_c(), if headless => {
            info!("Ctrl+C received — shutting down MCP HTTP server");
        }
    }
}

/// Start the MCP server on the streamable HTTP transport.
///
/// Binds a TCP listener on `addr` (expected to be loopback), mounts the rmcp
/// streamable HTTP service at `/mcp`, and wraps it with Bearer token
/// authentication read live from [`SharedState::read_config`]. Runs until the
/// process exits or the axum server encounters an unrecoverable error.
///
/// `headless` gates the Ctrl+C arm of the graceful-shutdown race (see
/// [`shutdown_signal`]): UI mode must not register an OS Ctrl+C handler, or
/// terminal interrupts would stop only the MCP transport while the window
/// lived on. `main.rs` passes the resolved run mode through.
///
/// The `receiver` slot is handed to the server so the `connect_bot` tool can
/// spawn a bot connection using the same receiver the UI path uses.
pub async fn serve_http(
    state: Arc<SharedState>,
    sender: BotCommandSender,
    receiver: ReceiverSlot,
    addr: SocketAddr,
    headless: bool,
) {
    // Keep a clone for status updates after `state` is moved into the router.
    let state_for_status = Arc::clone(&state);

    // The streamable HTTP service creates a fresh McpBotServer per session,
    // so capture cheap clones of state, sender, and receiver in the factory
    // closure.
    let state_for_factory = Arc::clone(&state);
    let sender_for_factory = sender.clone();
    let receiver_for_factory = Arc::clone(&receiver);
    let mcp_service: StreamableHttpService<McpBotServer, LocalSessionManager> =
        StreamableHttpService::new(
            move || {
                Ok::<_, std::io::Error>(McpBotServer::new(
                    Arc::clone(&state_for_factory),
                    sender_for_factory.clone(),
                    Arc::clone(&receiver_for_factory),
                ))
            },
            Arc::new(LocalSessionManager::default()),
            StreamableHttpServerConfig::default(),
        );

    let shutdown_token = state.shutdown_token();
    let middleware_state = Arc::clone(&state);
    let app = Router::new()
        .nest_service("/mcp", mcp_service)
        .layer(from_fn_with_state(middleware_state, bearer_auth_middleware))
        .with_state(state);

    let listener = match TcpListener::bind(addr).await {
        Ok(listener) => {
            state_for_status.set_mcp_server_status(McpServerStatus::Running(addr));
            info!(addr = %addr, "MCP HTTP server listening");
            listener
        }
        Err(e) => {
            let msg = format!("MCP HTTP bind failed: {e}");
            error!(error = %e, "Failed to bind MCP HTTP listener");
            state_for_status.set_mcp_server_status(McpServerStatus::Failed(msg.clone()));
            state_for_status.set_last_error(msg);
            return;
        }
    };

    info!("MCP server starting on HTTP at {addr}");

    if let Err(e) = axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_signal(shutdown_token, headless).await;
        })
        .await
    {
        let msg = format!("MCP HTTP server error: {e}");
        error!(error = %e, "MCP HTTP server error");
        state_for_status.set_mcp_server_status(McpServerStatus::Failed(msg.clone()));
        state_for_status.set_last_error(msg);
        return;
    }

    state_for_status.set_mcp_server_status(McpServerStatus::Stopped);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::create_command_channel;
    use crate::config::AppConfig;
    use axum::http::HeaderValue;
    use std::collections::HashSet;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    use tower::ServiceExt;

    /// Construct the same `(state, server)` pair every server test uses.
    fn make_server() -> (Arc<SharedState>, McpBotServer) {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, receiver) = create_command_channel(4, Arc::clone(&state));
        let receiver: ReceiverSlot = Arc::new(std::sync::Mutex::new(Some(receiver)));
        let server = McpBotServer::new(Arc::clone(&state), sender, receiver);
        (state, server)
    }

    /// Drive the REAL rmcp JSON-RPC transport over an in-memory duplex pair.
    /// Returns the JSON-RPC response for one `tools/call` request. This is
    /// the dispatch-layer coverage F-3 asked for: the request goes through
    /// `tool_router().call`, parameter deserialization, the `#[tool]` macro
    /// registration, and `BotError -> ErrorData` serialization — none of
    /// which is exercised by calling a handler method directly.
    async fn json_rpc_call(request: serde_json::Value) -> serde_json::Value {
        let (state, server) = make_server();
        let _ = state;

        let (server_io, client_io) = tokio::io::duplex(16 * 1024);
        let (server_r, server_w) = tokio::io::split(server_io);
        let transport =
            rmcp::transport::async_rw::AsyncRwTransport::<rmcp::RoleServer, _, _>::new_server(
                server_r, server_w,
            );
        let running = rmcp::service::serve_directly(server, transport, None);
        let cancel = running.cancellation_token();
        let handle = tokio::spawn(running.waiting());

        let (client_r, mut client_w) = tokio::io::split(client_io);
        let mut line = serde_json::to_vec(&request).expect("request serializes");
        line.push(10); // newline byte
        client_w.write_all(&line).await.expect("write request");
        client_w.flush().await.expect("flush request");

        let mut reader = tokio::io::BufReader::new(client_r);
        let mut reply = Vec::new();
        reader.read_until(10, &mut reply).await.expect("read reply");
        let response: serde_json::Value = serde_json::from_slice(&reply).expect("valid JSON reply");

        cancel.cancel();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), handle).await;
        response
    }

    /// F-3(a): the generated registry must contain the full tool surface with
    /// the annotations the MCP client relies on. If a `#[tool]` registration
    /// silently drops, this test goes red.
    #[test]
    fn test_tool_registry_lists_all_tools_with_annotations() {
        let tools = McpBotServer::tool_router().list_all();
        assert_eq!(
            tools.len(),
            41,
            "the registry snapshot must cover every #[tool]"
        );

        let names: HashSet<&str> = tools.iter().map(|tool| tool.name.as_ref()).collect();
        // Spot-check categories whose macro drift has burned us before.
        for name in [
            "get_self_info",
            "get_nearby_blocks",
            "send_chat",
            "execute_command",
            "break_block",
            "act",
            "update_settings",
            "connect_bot",
            "disconnect_bot",
        ] {
            assert!(names.contains(name), "missing tool registration: {name}");
        }

        let annotations = |name: &str| {
            tools
                .iter()
                .find(|tool| tool.name == name)
                .and_then(|tool| tool.annotations.clone())
                .unwrap_or_else(|| panic!("missing tool: {name}"))
        };
        assert_eq!(
            annotations("get_self_info").read_only_hint,
            Some(true),
            "read_only annotation must be preserved"
        );
        assert_eq!(
            annotations("send_chat").destructive_hint,
            Some(true),
            "destructive annotation must be preserved"
        );
        assert_eq!(
            annotations("execute_command").destructive_hint,
            Some(true),
            "destructive annotation must be preserved"
        );
    }

    /// 2026-08-25 review: the Creative-mode hint must be part of the
    /// REGISTERED tool descriptions clients actually see. The hint used to
    /// live in `tools_block.rs` constants that no registered description
    /// referenced — a test asserted the constants contained it while the
    /// wire contract never carried it.
    #[test]
    fn test_creative_hint_present_in_registered_block_tool_descriptions() {
        const CREATIVE_MODE_HINT: &str = "In Creative mode, prefer `execute_command` with `/fill` or `/setblock` for bulk building.";
        let tools = McpBotServer::tool_router().list_all();
        for name in ["break_block", "place_block"] {
            let tool = tools
                .iter()
                .find(|tool| tool.name == name)
                .unwrap_or_else(|| panic!("missing tool: {name}"));
            let description = tool
                .description
                .as_ref()
                .unwrap_or_else(|| panic!("{name} must carry a description"));
            assert!(
                description.contains(CREATIVE_MODE_HINT),
                "{name} registered description must contain the Creative-mode hint, got: {description}"
            );
        }
    }

    /// F-3(b): drive `tools/call` through the real JSON-RPC transport —
    /// happy path and an unknown-tool -32602.
    #[tokio::test]
    async fn test_call_tool_dispatch_layer_happy_path_and_invalid_tool() {
        let response = json_rpc_call(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "send_chat",
                "arguments": { "message": "hello" }
            }
        }))
        .await;
        assert_eq!(response["id"], 1);
        assert_eq!(
            response["error"]["code"], -32000,
            "offline error must round-trip: {response}"
        );
        assert_eq!(response["error"]["data"]["reason"], "bot_disconnected");

        let invalid = json_rpc_call(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "not_a_tool",
                "arguments": {}
            }
        }))
        .await;
        assert_eq!(invalid["id"], 2);
        assert_eq!(
            invalid["error"]["code"], -32602,
            "unknown tool must be INVALID_PARAMS: {invalid}"
        );
    }

    /// F-3(c): exercise the axum wrapper around `bearer_auth_middleware`
    /// (header extraction + 401 response shape) with `tower::ServiceExt::oneshot`.
    /// The pure decision function is unit-tested elsewhere; this covers the
    /// part a real HTTP client hits.
    #[tokio::test]
    async fn test_bearer_auth_middleware_http_wrapper_401_shape() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        state.update_config(|cfg| {
            cfg.mcp_auth_enabled = true;
            cfg.mcp_token = "secret".into();
        });

        let app: axum::Router = axum::Router::new()
            .route("/", axum::routing::get(|| async { "ok" }))
            .layer(from_fn_with_state(
                Arc::clone(&state),
                bearer_auth_middleware,
            ))
            .with_state(state);

        let unauthorized = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/")
                    .body(axum::body::Body::empty())
                    .expect("valid request"),
            )
            .await
            .expect("router call");
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
        let bytes = axum::body::to_bytes(unauthorized.into_body(), 1024)
            .await
            .expect("body readable");
        assert_eq!(&bytes[..], b"Unauthorized");

        let authorized = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/")
                    .header("authorization", "Bearer secret")
                    .body(axum::body::Body::empty())
                    .expect("valid request"),
            )
            .await
            .expect("router call");
        assert_eq!(authorized.status(), StatusCode::OK);
    }

    /// F-3(b): malformed arguments must surface the rmcp deserialization
    /// error through the transport, not panic or hang.
    #[tokio::test]
    async fn test_call_tool_dispatch_layer_malformed_arguments() {
        let response = json_rpc_call(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "move_to",
                "arguments": { "x": 1 }
            }
        }))
        .await;
        assert_eq!(response["id"], 3);
        // rmcp surfaces argument-deserialization failures as an
        // `isError: true` tool result (spec-recommended tool-level failure),
        // unlike BotError which maps to a JSON-RPC error per F-32.
        assert_eq!(response["result"]["isError"], true, "got: {response}");
        assert!(
            response["result"]["content"][0]["text"]
                .as_str()
                .is_some_and(|text| text.contains("missing field")),
            "got: {response}"
        );
    }

    /// `shutdown_signal` resolves when the shutdown token is cancelled —
    /// this is the path the UI (window close) and headless stdio client exit
    /// rely on. The Ctrl+C branch cannot be exercised in CI (no terminal
    /// signal injection), so only the token branch is covered here, for
    /// both gate values (the `headless=false` arm must still resolve on the
    /// token alone).
    #[tokio::test]
    async fn test_shutdown_signal_returns_on_token_cancel() {
        for headless in [true, false] {
            let token = CancellationToken::new();
            let token_for_test = token.clone();
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                token_for_test.cancel();
            });
            // Must resolve (not hang) once the token fires.
            shutdown_signal(token, headless).await;
        }
    }

    /// The headless idle watchdog must NOT fire while the session is younger
    /// than the timeout (a fresh process gets a full grace period even
    /// before its first request).
    #[test]
    fn test_is_headless_idle_grace_period_before_first_command() {
        let timeout = Duration::from_secs(600);
        let started = Instant::now();
        // No request ever received (None) but only 60 seconds after start:
        // not idle yet.
        assert!(!is_headless_idle(
            started + Duration::from_secs(60),
            None,
            started,
            timeout
        ));
        // 10 minutes elapsed with zero requests: abandoned — idle.
        assert!(is_headless_idle(
            started + Duration::from_secs(600),
            None,
            started,
            timeout
        ));
    }

    /// Recent request activity resets the idle clock.
    #[test]
    fn test_is_headless_idle_recent_activity_resets() {
        let timeout = Duration::from_secs(600);
        let started = Instant::now();
        let now = started + Duration::from_secs(1_800);
        // 30 minutes after start, but a request arrived 5 minutes ago.
        assert!(!is_headless_idle(
            now,
            Some(started + Duration::from_secs(1_500)),
            started,
            timeout
        ));
        // Same session, last request 11 minutes ago: idle.
        assert!(is_headless_idle(
            now,
            Some(started + Duration::from_secs(1_140)),
            started,
            timeout
        ));
    }

    /// A clock "jump" can never make the decision fire early. With
    /// monotonic stamps a backward jump is impossible by construction; the
    /// only remaining anomaly is a caller passing an anchor in the future,
    /// which `saturating_duration_since` resolves to zero elapsed — never
    /// idle.
    #[test]
    fn test_is_headless_idle_clock_anomaly_safe() {
        let timeout = Duration::from_secs(600);
        let now = Instant::now();
        // Anchor ahead of `now` (the old epoch-ms backward-jump case):
        // elapsed since the anchor saturates to zero → not idle.
        assert!(!is_headless_idle(
            now,
            None,
            now + Duration::from_secs(1),
            timeout
        ));
        assert!(!is_headless_idle(
            now,
            Some(now + Duration::from_secs(1)),
            now + Duration::from_secs(1),
            timeout
        ));
    }

    /// Verify get_info() returns the expected server name.
    #[test]
    fn test_get_info_server_name() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, receiver) = create_command_channel(4, Arc::clone(&state));
        let receiver: ReceiverSlot = Arc::new(std::sync::Mutex::new(Some(receiver)));
        let server = McpBotServer::new(state, sender, receiver);

        let info = server.get_info();
        assert_eq!(info.server_info.name, "minecraft-mcp-rs");
    }

    /// Verify get_info() returns the Cargo package version.
    #[test]
    fn test_get_info_version() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, receiver) = create_command_channel(4, Arc::clone(&state));
        let receiver: ReceiverSlot = Arc::new(std::sync::Mutex::new(Some(receiver)));
        let server = McpBotServer::new(state, sender, receiver);

        let info = server.get_info();
        assert_eq!(info.server_info.version, env!("CARGO_PKG_VERSION"));
    }

    /// Verify get_info() has tools enabled in capabilities.
    #[test]
    fn test_get_info_tools_enabled() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, receiver) = create_command_channel(4, Arc::clone(&state));
        let receiver: ReceiverSlot = Arc::new(std::sync::Mutex::new(Some(receiver)));
        let server = McpBotServer::new(state, sender, receiver);

        let info = server.get_info();
        assert!(
            info.capabilities.tools.is_some(),
            "tools capability should be enabled"
        );
    }

    /// Verify get_info() includes instructions.
    #[test]
    fn test_get_info_has_instructions() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, receiver) = create_command_channel(4, Arc::clone(&state));
        let receiver: ReceiverSlot = Arc::new(std::sync::Mutex::new(Some(receiver)));
        let server = McpBotServer::new(state, sender, receiver);

        let info = server.get_info();
        assert!(info.instructions.is_some());
        assert!(info.instructions.unwrap().contains("Minecraft"));
    }

    /// Movement tool integration tests — verify offline rejection.
    #[tokio::test]
    async fn test_movement_tools_offline() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, receiver) = create_command_channel(4, Arc::clone(&state));
        let receiver: ReceiverSlot = Arc::new(std::sync::Mutex::new(Some(receiver)));
        let server = McpBotServer::new(state, sender, receiver);

        let result = server
            .move_to(Parameters(MoveToInput { x: 0, y: 64, z: 0 }))
            .await;
        assert!(matches!(result, Err(BotError::Offline(_))));

        let result = server
            .walk_direction(Parameters(WalkDirectionInput {
                direction: "north".into(),
                distance: 1,
            }))
            .await;
        assert!(matches!(result, Err(BotError::Offline(_))));

        let result = server.jump(Parameters(JumpInput {})).await;
        assert!(matches!(result, Err(BotError::Offline(_))));

        let result = server
            .teleport(Parameters(TeleportInput { x: 0, y: 64, z: 0 }))
            .await;
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    /// Container tool integration tests — verify offline/no-container rejection.
    #[tokio::test]
    async fn test_container_tools_offline() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, receiver) = create_command_channel(4, Arc::clone(&state));
        let receiver: ReceiverSlot = Arc::new(std::sync::Mutex::new(Some(receiver)));
        let server = McpBotServer::new(state, sender, receiver);

        let result = server
            .open_container(Parameters(OpenContainerInput { x: 0, y: 64, z: 0 }))
            .await;
        assert!(matches!(result, Err(BotError::Offline(_))));

        // Parameter validation passes (slot 0, count 1 are valid), then
        // offline check runs before container-open check.
        let result = server
            .take_from_container(Parameters(TakeFromContainerInput {
                slot: 0,
                count: Some(1),
            }))
            .await;
        assert!(matches!(result, Err(BotError::Offline(_))));

        let result = server
            .put_into_container(Parameters(PutIntoContainerInput {
                slot: 0,
                count: Some(1),
            }))
            .await;
        assert!(matches!(result, Err(BotError::Offline(_))));

        let result = server
            .close_container(Parameters(CloseContainerInput {}))
            .await;
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    /// Combat tool integration tests — verify offline/entity-not-found rejection.
    #[tokio::test]
    async fn test_combat_tools_offline() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, receiver) = create_command_channel(4, Arc::clone(&state));
        let receiver: ReceiverSlot = Arc::new(std::sync::Mutex::new(Some(receiver)));
        let server = McpBotServer::new(state, sender, receiver);

        // Offline check comes before entity existence check, so even with
        // a valid entity_id we should get Offline error when bot is not connected.
        let result = server
            .attack_entity(Parameters(AttackEntityInput { entity_id: 42 }))
            .await;
        assert!(matches!(result, Err(BotError::Offline(_))));

        let result = server
            .shield_block(Parameters(ShieldBlockInput { blocking: true }))
            .await;
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    /// Query tools return offline error when the bot is not connected.
    #[tokio::test]
    async fn test_query_tools_offline() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, receiver) = create_command_channel(4, Arc::clone(&state));
        let receiver: ReceiverSlot = Arc::new(std::sync::Mutex::new(Some(receiver)));
        let server = McpBotServer::new(state, sender, receiver);

        assert!(matches!(
            server
                .get_self_info(Parameters(SelfInfoInput { force: false }))
                .await,
            Err(BotError::Offline(_))
        ));
        assert!(matches!(
            server
                .get_inventory(Parameters(InventoryInput { force: false }))
                .await,
            Err(BotError::Offline(_))
        ));
        assert!(matches!(
            server
                .get_nearby_blocks(Parameters(NearbyBlocksInput {
                    radius: 10,
                    filter_type: None,
                    top_only: false,
                    max_blocks: 500,
                }))
                .await,
            Err(BotError::Offline(_))
        ));
        assert!(matches!(
            server
                .get_nearby_entities(Parameters(NearbyEntitiesInput {
                    radius: 10,
                    max_entities: 500,
                }))
                .await,
            Err(BotError::Offline(_))
        ));
        assert!(matches!(
            server.get_chunk_summary().await,
            Err(BotError::Offline(_))
        ));
    }

    /// is_connected returns false when bot is offline.
    #[tokio::test]
    async fn test_is_connected_offline() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, receiver) = create_command_channel(4, Arc::clone(&state));
        let receiver: ReceiverSlot = Arc::new(std::sync::Mutex::new(Some(receiver)));
        let server = McpBotServer::new(state, sender, receiver);

        assert_eq!(
            server.is_connected().await.unwrap(),
            r#"{"connected":false}"#
        );
    }

    /// is_connected returns true when bot is online.
    #[tokio::test]
    async fn test_is_connected_online() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        state.set_online(true);
        let (sender, receiver) = create_command_channel(4, Arc::clone(&state));
        let receiver: ReceiverSlot = Arc::new(std::sync::Mutex::new(Some(receiver)));
        let server = McpBotServer::new(state, sender, receiver);

        assert_eq!(
            server.is_connected().await.unwrap(),
            r#"{"connected":true}"#
        );
    }

    // ── Block tool integration tests ───────────────────────────────────

    #[tokio::test]
    async fn test_break_block_offline_via_server() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, receiver) = create_command_channel(4, Arc::clone(&state));
        let receiver: ReceiverSlot = Arc::new(std::sync::Mutex::new(Some(receiver)));
        let server = McpBotServer::new(state, sender, receiver);

        let result = server
            .break_block(Parameters(BreakBlockInput {
                x: 0,
                y: 64,
                z: 0,
                use_best_tool: None,
            }))
            .await;
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    #[tokio::test]
    async fn test_place_block_offline_via_server() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, receiver) = create_command_channel(4, Arc::clone(&state));
        let receiver: ReceiverSlot = Arc::new(std::sync::Mutex::new(Some(receiver)));
        let server = McpBotServer::new(state, sender, receiver);

        let result = server
            .place_block(Parameters(PlaceBlockInput {
                x: 0,
                y: 64,
                z: 0,
                item_slot: 1,
            }))
            .await;
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    #[tokio::test]
    async fn test_use_item_on_block_offline_via_server() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, receiver) = create_command_channel(4, Arc::clone(&state));
        let receiver: ReceiverSlot = Arc::new(std::sync::Mutex::new(Some(receiver)));
        let server = McpBotServer::new(state, sender, receiver);

        let result = server
            .use_item_on_block(Parameters(UseItemOnBlockInput {
                x: 0,
                y: 64,
                z: 0,
                item_slot: None,
                face: None,
            }))
            .await;
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    #[tokio::test]
    async fn test_break_block_invalid_coords_via_server() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        state.set_online(true);
        let (sender, receiver) = create_command_channel(4, Arc::clone(&state));
        let receiver: ReceiverSlot = Arc::new(std::sync::Mutex::new(Some(receiver)));
        let server = McpBotServer::new(state, sender, receiver);

        let result = server
            .break_block(Parameters(BreakBlockInput {
                x: 0,
                y: 500,
                z: 0,
                use_best_tool: None,
            }))
            .await;
        assert!(matches!(result, Err(BotError::InvalidParams(ref msg))
                if msg.contains("out of bounds") || msg.contains("out of range")));
    }

    #[tokio::test]
    async fn test_place_block_invalid_slot_via_server() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        state.set_online(true);
        let (sender, receiver) = create_command_channel(4, Arc::clone(&state));
        let receiver: ReceiverSlot = Arc::new(std::sync::Mutex::new(Some(receiver)));
        let server = McpBotServer::new(state, sender, receiver);

        let result = server
            .place_block(Parameters(PlaceBlockInput {
                x: 0,
                y: 64,
                z: 0,
                item_slot: 9,
            }))
            .await;
        assert!(matches!(result, Err(BotError::InvalidParams(ref msg))
                if msg.contains("must be 0-8")));
    }

    #[tokio::test]
    async fn test_use_item_on_block_invalid_slot_via_server() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        state.set_online(true);
        let (sender, receiver) = create_command_channel(4, Arc::clone(&state));
        let receiver: ReceiverSlot = Arc::new(std::sync::Mutex::new(Some(receiver)));
        let server = McpBotServer::new(state, sender, receiver);

        let result = server
            .use_item_on_block(Parameters(UseItemOnBlockInput {
                x: 0,
                y: 64,
                z: 0,
                item_slot: Some(10),
                face: None,
            }))
            .await;
        assert!(matches!(result, Err(BotError::InvalidParams(ref msg))
                if msg.contains("must be 0-8")));
    }

    // ── Item tool integration tests ─────────────────────────────────────

    #[tokio::test]
    async fn test_switch_hotbar_slot_offline_via_server() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, receiver) = create_command_channel(4, Arc::clone(&state));
        let receiver: ReceiverSlot = Arc::new(std::sync::Mutex::new(Some(receiver)));
        let server = McpBotServer::new(state, sender, receiver);

        let result = server
            .switch_hotbar_slot(Parameters(SwitchHotbarSlotInput { slot: 0 }))
            .await;
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    #[tokio::test]
    async fn test_drop_item_offline_via_server() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, receiver) = create_command_channel(4, Arc::clone(&state));
        let receiver: ReceiverSlot = Arc::new(std::sync::Mutex::new(Some(receiver)));
        let server = McpBotServer::new(state, sender, receiver);

        let result = server
            .drop_item(Parameters(DropItemInput {
                slot: 0,
                count: Some(1),
            }))
            .await;
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    #[tokio::test]
    async fn test_use_item_offline_via_server() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, receiver) = create_command_channel(4, Arc::clone(&state));
        let receiver: ReceiverSlot = Arc::new(std::sync::Mutex::new(Some(receiver)));
        let server = McpBotServer::new(state, sender, receiver);

        let result = server
            .use_item(Parameters(UseItemInput { item_slot: None }))
            .await;
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    #[tokio::test]
    async fn test_get_hotbar_offline_via_server() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, receiver) = create_command_channel(4, Arc::clone(&state));
        let receiver: ReceiverSlot = Arc::new(std::sync::Mutex::new(Some(receiver)));
        let server = McpBotServer::new(state, sender, receiver);

        let result = server
            .get_hotbar(Parameters(HotbarInput { force: false }))
            .await;
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    #[tokio::test]
    async fn test_get_bot_status_offline_via_server_reports_disconnected() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, receiver) = create_command_channel(4, Arc::clone(&state));
        let receiver: ReceiverSlot = Arc::new(std::sync::Mutex::new(Some(receiver)));
        let server = McpBotServer::new(state, sender, receiver);

        let result = server
            .get_bot_status(Parameters(BotStatusInput { force: false }))
            .await
            .expect("get_bot_status never errors offline");
        assert!(result.contains("false"));
    }

    #[tokio::test]
    async fn test_give_item_offline_via_server() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, receiver) = create_command_channel(4, Arc::clone(&state));
        let receiver: ReceiverSlot = Arc::new(std::sync::Mutex::new(Some(receiver)));
        let server = McpBotServer::new(state, sender, receiver);

        let result = server
            .give_item(Parameters(GiveItemInput {
                item_id: "dirt".into(),
                count: None,
                target: None,
                hotbar_slot: None,
            }))
            .await;
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    #[tokio::test]
    async fn test_set_hotbar_item_offline_via_server() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, receiver) = create_command_channel(4, Arc::clone(&state));
        let receiver: ReceiverSlot = Arc::new(std::sync::Mutex::new(Some(receiver)));
        let server = McpBotServer::new(state, sender, receiver);

        let result = server
            .set_hotbar_item(Parameters(SetHotbarItemInput {
                hotbar_slot: 0,
                item_id: "dirt".into(),
                count: Some(1),
            }))
            .await;
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    #[tokio::test]
    async fn test_equip_tool_offline_via_server() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, receiver) = create_command_channel(4, Arc::clone(&state));
        let receiver: ReceiverSlot = Arc::new(std::sync::Mutex::new(Some(receiver)));
        let server = McpBotServer::new(state, sender, receiver);

        let result = server
            .equip_tool(Parameters(EquipToolInput {
                tool_type: "pickaxe".into(),
                material_preference: None,
            }))
            .await;
        assert!(matches!(result, Err(BotError::Offline(_))));
    }

    #[tokio::test]
    async fn test_switch_hotbar_slot_invalid_via_server() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        state.set_online(true);
        let (sender, receiver) = create_command_channel(4, Arc::clone(&state));
        let receiver: ReceiverSlot = Arc::new(std::sync::Mutex::new(Some(receiver)));
        let server = McpBotServer::new(state, sender, receiver);

        let result = server
            .switch_hotbar_slot(Parameters(SwitchHotbarSlotInput { slot: 9 }))
            .await;
        assert!(matches!(result, Err(BotError::InvalidParams(ref msg))
                if msg.contains("must be 0-8")));
    }

    #[tokio::test]
    async fn test_equip_tool_unknown_type_via_server() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        state.set_online(true);
        let (sender, receiver) = create_command_channel(4, Arc::clone(&state));
        let receiver: ReceiverSlot = Arc::new(std::sync::Mutex::new(Some(receiver)));
        let server = McpBotServer::new(state, sender, receiver);

        let result = server
            .equip_tool(Parameters(EquipToolInput {
                tool_type: "invalid_tool".into(),
                material_preference: None,
            }))
            .await;
        assert!(matches!(result, Err(BotError::InvalidParams(ref msg))
                if msg.contains("Unknown tool type")));
    }

    // ── Bearer token helper tests ───────────────────────────────────────

    #[test]
    fn test_extract_bearer_token_valid() {
        assert_eq!(extract_bearer_token("Bearer secret"), Some("secret"));
        assert_eq!(
            extract_bearer_token("Bearer minecraft-mcp-rs"),
            Some("minecraft-mcp-rs")
        );
    }

    #[test]
    fn test_extract_bearer_token_trims_whitespace() {
        assert_eq!(extract_bearer_token("Bearer  token  "), Some("token"));
    }

    #[test]
    fn test_extract_bearer_token_rejects_missing_prefix() {
        assert_eq!(extract_bearer_token("secret"), None);
        assert_eq!(extract_bearer_token("Basic secret"), None);
        assert_eq!(extract_bearer_token("bearer secret"), None);
    }

    #[test]
    fn test_is_bearer_authorized_accepts_valid_token() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let expected = state.read_config().mcp_token.clone();

        let mut headers = HeaderMap::new();
        headers.insert(
            "Authorization",
            format!("Bearer {expected}").parse().unwrap(),
        );
        assert!(is_bearer_authorized(&headers, &expected));
    }

    #[test]
    fn test_is_bearer_authorized_rejects_wrong_token() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let expected = state.read_config().mcp_token.clone();

        let mut headers = HeaderMap::new();
        headers.insert("Authorization", "Bearer wrong".parse().unwrap());
        assert!(!is_bearer_authorized(&headers, &expected));
    }

    #[test]
    fn test_is_bearer_authorized_rejects_missing_header() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let expected = state.read_config().mcp_token.clone();

        let headers = HeaderMap::new();
        assert!(!is_bearer_authorized(&headers, &expected));
    }

    #[test]
    fn test_is_bearer_authorized_rejects_empty_token_when_configured() {
        // When a token is configured, an empty Bearer token must be rejected.
        let expected_token = "secret-token";
        let mut headers = HeaderMap::new();
        headers.insert("Authorization", "Bearer ".parse().unwrap());
        assert!(!is_bearer_authorized(&headers, expected_token));

        // Also reject "Bearer" with no token at all.
        let mut headers2 = HeaderMap::new();
        headers2.insert("Authorization", "Bearer".parse().unwrap());
        assert!(!is_bearer_authorized(&headers2, expected_token));
    }

    // ── constant_time_eq tests ─────────────────────────────────────────

    #[test]
    fn test_constant_time_eq_equal_strings() {
        assert!(constant_time_eq("secret-token", "secret-token"));
        assert!(constant_time_eq("a", "a"));
    }

    #[test]
    fn test_constant_time_eq_unequal_same_length() {
        // Mismatch only in the last byte — a naive `==` would short-circuit
        // late; the constant-time comparison must still return false.
        assert!(!constant_time_eq("secret-token", "secret-tokeN"));
        assert!(!constant_time_eq("aaaa", "aaab"));
        // Mismatch in the first byte.
        assert!(!constant_time_eq("xaaa", "ybbb"));
    }

    #[test]
    fn test_constant_time_eq_different_lengths() {
        assert!(!constant_time_eq("short", "much-longer-token"));
        assert!(!constant_time_eq("abc", "ab"));
    }

    #[test]
    fn test_constant_time_eq_both_empty() {
        // Equal lengths (zero) and an empty XOR accumulation → true.
        // This does NOT weaken auth: `is_bearer_authorized` rejects empty
        // client tokens via its `!token.is_empty()` guard before comparing
        // (see test_is_bearer_authorized_empty_header_token_rejected).
        assert!(constant_time_eq("", ""));
    }

    #[test]
    fn test_constant_time_eq_nonempty_vs_empty() {
        assert!(!constant_time_eq("token", ""));
        assert!(!constant_time_eq("", "token"));
    }

    #[test]
    fn test_is_bearer_authorized_empty_header_token_rejected() {
        // `constant_time_eq("", "")` is true, but an empty Bearer token must
        // still fail authentication when a token is configured — the
        // `!token.is_empty()` guard in `is_bearer_authorized` enforces this.
        assert!(constant_time_eq("", ""));
        let mut headers = HeaderMap::new();
        headers.insert("Authorization", "Bearer ".parse().unwrap());
        assert!(!is_bearer_authorized(&headers, "expected-secret"));
    }

    // ── is_request_authorized pure-function tests ───────────────────────

    /// Auth disabled: every request passes, even with no/mismatched header.
    /// The auth-ON code path must be short-circuited BEFORE any token
    /// comparison (no timing side channel is reachable when auth is off).
    #[test]
    fn test_is_request_authorized_off_passes() {
        let headers = HeaderMap::new();
        assert!(is_request_authorized(false, "sekrit", &headers));

        // Also verify a fully-present token that would match is moot when
        // auth is disabled — auth-off bypasses the whole comparison.
        let mut headers = HeaderMap::new();
        headers.insert("Authorization", HeaderValue::from_static("Bearer sekrit"));
        assert!(is_request_authorized(false, "sekrit", &headers));
    }

    /// Auth enabled with an empty configured token: reject ALL requests.
    /// An empty expected token is a misconfiguration (validate() forbids it
    /// when auth is on) and must fail closed, never open.
    #[test]
    fn test_is_request_authorized_on_empty_token_rejects() {
        let mut headers = HeaderMap::new();
        headers.insert("Authorization", HeaderValue::from_static("Bearer sekrit"));
        assert!(!is_request_authorized(true, "", &headers));

        // Even a matching-looking empty bearer token cannot pass.
        let mut headers2 = HeaderMap::new();
        headers2.insert("Authorization", HeaderValue::from_static("Bearer "));
        assert!(!is_request_authorized(true, "", &headers2));
    }

    /// Auth enabled + correct token: passes.
    #[test]
    fn test_is_request_authorized_on_correct_token_passes() {
        let mut headers = HeaderMap::new();
        headers.insert("Authorization", HeaderValue::from_static("Bearer sekrit"));
        assert!(is_request_authorized(true, "sekrit", &headers));
    }

    /// Auth enabled + wrong token: rejected (constant-time compare).
    #[test]
    fn test_is_request_authorized_on_wrong_token_rejects() {
        let mut headers = HeaderMap::new();
        headers.insert("Authorization", HeaderValue::from_static("Bearer wrong"));
        assert!(!is_request_authorized(true, "sekrit", &headers));
    }

    /// Auth enabled + non-empty token + no Authorization header: rejected.
    #[test]
    fn test_is_request_authorized_on_missing_header_rejects() {
        let headers = HeaderMap::new();
        assert!(!is_request_authorized(true, "sekrit", &headers));
    }
}
