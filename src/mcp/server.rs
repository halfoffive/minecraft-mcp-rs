//! MCP server setup, transport, and request dispatch.
//!
//! Uses rmcp 1.7.0 with `#[tool_router]`/`#[tool_handler]` macros to define
//! 25+ MCP tools. All logging goes to stderr via `tracing`.

use std::sync::Arc;

use rmcp::{
    tool, tool_handler, tool_router,
    handler::server::wrapper::Parameters,
    model::{Implementation, ServerCapabilities, ServerInfo},
    ServerHandler, ServiceExt,
    transport::io::stdio,
};
use tracing::{error, info};

use crate::channel::BotCommandSender;
use crate::mcp::tools_block::{
    BreakBlockInput, PlaceBlockInput, UseItemOnBlockInput,
};
use crate::mcp::tools_chat::{ExecuteCommandInput, SendChatInput, SetGameModeInput};
use crate::mcp::tools_combat::{AttackEntityInput, ShieldBlockInput};
use crate::mcp::tools_container::{
    CloseContainerInput, OpenContainerInput, PutIntoContainerInput,
    TakeFromContainerInput,
};
use crate::mcp::tools_item::{
    DropItemInput, EquipToolInput, SwitchHotbarSlotInput, UseItemInput,
};
use crate::mcp::tools_movement::{JumpInput, MoveToInput, TeleportInput, WalkDirectionInput};
use crate::state::SharedState;

// ---------------------------------------------------------------------------
// McpBotServer
// ---------------------------------------------------------------------------

/// MCP server struct holding shared state and the bot command channel.
///
/// The `Arc<SharedState>` is read directly by query tools; action tools
/// send [`BotCommand`](crate::types::BotCommand) through the sender.
pub struct McpBotServer {
    state: Arc<SharedState>,
    sender: BotCommandSender,
}

impl McpBotServer {
    /// Create a new MCP server instance.
    pub fn new(state: Arc<SharedState>, sender: BotCommandSender) -> Self {
        Self { state, sender }
    }
}

// ---------------------------------------------------------------------------
// Tool Router — 26 MCP tool stubs
// ---------------------------------------------------------------------------

#[tool_router]
impl McpBotServer {
    // ── Query tools (read_only) ──────────────────────────────
    //
    // NOTE: get_nearby_blocks and get_nearby_entities currently take no
    // parameters due to a schemars version mismatch (Cargo.toml uses
    // schemars 0.8, rmcp re-exports schemars 1.x).  Once the project
    // upgrades to schemars 1.x, these tools should accept `radius: u32`
    // and `filter_type: Option<String>` parameters.  For now sensible
    // defaults (radius=10, no filter) are baked into the implementations.

    #[tool(description = "Get information about the bot's own player", annotations(read_only_hint = true))]
    async fn get_self_info(&self) -> String {
        crate::mcp::tools_query::get_self_info(&self.state)
    }

    #[tool(description = "Get the bot's inventory contents", annotations(read_only_hint = true))]
    async fn get_inventory(&self) -> String {
        crate::mcp::tools_query::get_inventory(&self.state)
    }

    #[tool(description = "Get blocks near the bot's position (radius=10, no filter)", annotations(read_only_hint = true))]
    async fn get_nearby_blocks(&self) -> String {
        crate::mcp::tools_query::get_nearby_blocks(&self.state, 10, None)
    }

    #[tool(description = "Get entities near the bot's position (radius=10)", annotations(read_only_hint = true))]
    async fn get_nearby_entities(&self) -> String {
        crate::mcp::tools_query::get_nearby_entities(&self.state, 10)
    }

    #[tool(description = "Get a summary of loaded chunks", annotations(read_only_hint = true))]
    async fn get_chunk_summary(&self) -> String {
        crate::mcp::tools_query::get_chunk_summary(&self.state)
    }

    #[tool(description = "Check if the bot is connected to a Minecraft server", annotations(read_only_hint = true))]
    async fn is_connected(&self) -> String {
        crate::mcp::tools_query::is_connected(&self.state)
    }

    // ── Movement tools ───────────────────────────────────────

    #[tool(description = "Move the bot to a specific position")]
    async fn move_to(
        &self,
        Parameters(input): Parameters<MoveToInput>,
    ) -> String {
        crate::mcp::tools_movement::handle_move_to(&self.state, &self.sender, input).await
    }

    #[tool(description = "Walk the bot in a cardinal direction")]
    async fn walk_direction(
        &self,
        Parameters(input): Parameters<WalkDirectionInput>,
    ) -> String {
        crate::mcp::tools_movement::handle_walk_direction(&self.state, &self.sender, input).await
    }

    #[tool(description = "Make the bot jump")]
    async fn jump(
        &self,
        Parameters(input): Parameters<JumpInput>,
    ) -> String {
        crate::mcp::tools_movement::handle_jump(&self.state, &self.sender, input).await
    }

    #[tool(description = "Teleport the bot to a position (requires Creative mode)")]
    async fn teleport(
        &self,
        Parameters(input): Parameters<TeleportInput>,
    ) -> String {
        crate::mcp::tools_movement::handle_teleport(&self.state, &self.sender, input).await
    }

    // ── Block tools (destructive) ────────────────────────────

    #[tool(description = "Break a block at the given position", annotations(destructive_hint = true))]
    async fn break_block(
        &self,
        Parameters(input): Parameters<BreakBlockInput>,
    ) -> String {
        crate::mcp::tools_block::handle_break_block(&self.state, &self.sender, input).await
    }

    #[tool(description = "Place a block at the given position", annotations(destructive_hint = true))]
    async fn place_block(
        &self,
        Parameters(input): Parameters<PlaceBlockInput>,
    ) -> String {
        crate::mcp::tools_block::handle_place_block(&self.state, &self.sender, input).await
    }

    #[tool(description = "Use the held item on a block", annotations(destructive_hint = true))]
    async fn use_item_on_block(
        &self,
        Parameters(input): Parameters<UseItemOnBlockInput>,
    ) -> String {
        crate::mcp::tools_block::handle_use_item_on_block(&self.state, &self.sender, input).await
    }

    // ── Item tools (destructive) ─────────────────────────────

    #[tool(description = "Switch to a hotbar slot (0-8).", annotations(destructive_hint = true))]
    async fn switch_hotbar_slot(
        &self,
        Parameters(input): Parameters<SwitchHotbarSlotInput>,
    ) -> String {
        crate::mcp::tools_item::handle_switch_hotbar_slot(&self.state, &self.sender, input).await
    }

    #[tool(description = "Drop items from an inventory slot.", annotations(destructive_hint = true))]
    async fn drop_item(
        &self,
        Parameters(input): Parameters<DropItemInput>,
    ) -> String {
        crate::mcp::tools_item::handle_drop_item(&self.state, &self.sender, input).await
    }

    #[tool(description = "Use the currently held item.", annotations(destructive_hint = true))]
    async fn use_item(
        &self,
        Parameters(input): Parameters<UseItemInput>,
    ) -> String {
        crate::mcp::tools_item::handle_use_item(&self.state, &self.sender, input).await
    }

    #[tool(description = "Equip the best available tool of a given type.", annotations(destructive_hint = true))]
    async fn equip_tool(
        &self,
        Parameters(input): Parameters<EquipToolInput>,
    ) -> String {
        crate::mcp::tools_item::handle_equip_tool(&self.state, &self.sender, input).await
    }

    // ── Container tools (destructive) ────────────────────────

    #[tool(description = "Open a container at the given position", annotations(destructive_hint = true))]
    async fn open_container(
        &self,
        Parameters(input): Parameters<OpenContainerInput>,
    ) -> String {
        crate::mcp::tools_container::handle_open_container(&self.state, &self.sender, input).await
    }

    #[tool(description = "Take items from an open container slot", annotations(destructive_hint = true))]
    async fn take_from_container(
        &self,
        Parameters(input): Parameters<TakeFromContainerInput>,
    ) -> String {
        crate::mcp::tools_container::handle_take_from_container(&self.state, &self.sender, input).await
    }

    #[tool(description = "Put items into an open container slot", annotations(destructive_hint = true))]
    async fn put_into_container(
        &self,
        Parameters(input): Parameters<PutIntoContainerInput>,
    ) -> String {
        crate::mcp::tools_container::handle_put_into_container(&self.state, &self.sender, input).await
    }

    #[tool(description = "Close the currently open container", annotations(destructive_hint = true))]
    async fn close_container(
        &self,
        Parameters(input): Parameters<CloseContainerInput>,
    ) -> String {
        crate::mcp::tools_container::handle_close_container(&self.state, &self.sender, input).await
    }

    // ── Combat / Chat tools (destructive) ────────────────────

    #[tool(description = "Attack an entity by its Minecraft entity ID", annotations(destructive_hint = true))]
    async fn attack_entity(
        &self,
        Parameters(input): Parameters<AttackEntityInput>,
    ) -> String {
        crate::mcp::tools_combat::handle_attack_entity(&self.state, &self.sender, input).await
    }

    #[tool(description = "Hold up shield to block incoming attacks", annotations(destructive_hint = true))]
    async fn shield_block(
        &self,
        Parameters(input): Parameters<ShieldBlockInput>,
    ) -> String {
        crate::mcp::tools_combat::handle_shield_block(&self.state, &self.sender, input).await
    }

    #[tool(description = "Send a chat message to the server", annotations(destructive_hint = true))]
    async fn send_chat(
        &self,
        Parameters(SendChatInput { message }): Parameters<SendChatInput>,
    ) -> String {
        crate::mcp::tools_chat::handle_send_chat(&self.sender, message).await
    }

    #[tool(description = "Execute a Minecraft command (requires op). The / prefix is auto-added if omitted.", annotations(destructive_hint = true))]
    async fn execute_command(
        &self,
        Parameters(ExecuteCommandInput { command }): Parameters<ExecuteCommandInput>,
    ) -> String {
        crate::mcp::tools_chat::handle_execute_command(&self.sender, command).await
    }

    #[tool(description = "Set the bot's game mode (requires OP permissions). Valid modes: survival, creative, adventure, spectator.", annotations(destructive_hint = true))]
    async fn set_game_mode(
        &self,
        Parameters(SetGameModeInput { mode }): Parameters<SetGameModeInput>,
    ) -> String {
        crate::mcp::tools_chat::handle_set_game_mode(&self.sender, mode).await
    }
}

// ---------------------------------------------------------------------------
// ServerHandler — auto-generated call_tool / list_tools / get_info
// ---------------------------------------------------------------------------

#[tool_handler]
impl ServerHandler for McpBotServer {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.server_info =
            Implementation::new("minecraft-mcp-rs", env!("CARGO_PKG_VERSION"));
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.instructions = Some(
            "Minecraft bot control via MCP. Use query tools to inspect world state, \
             action tools to control the bot. All destructive operations are annotated."
                .into(),
        );
        info
    }
}

// ---------------------------------------------------------------------------
// Server entry point
// ---------------------------------------------------------------------------

/// Start the MCP server on stdio transport.
///
/// This function blocks until the transport is closed. All logging goes to
/// stderr; stdout is reserved for MCP JSON-RPC messages.
pub async fn serve_stdio(state: Arc<SharedState>, sender: BotCommandSender) {
    let server = McpBotServer::new(state, sender);
    let (stdin, stdout) = stdio();

    info!("MCP server starting on stdio");

    match server.serve((stdin, stdout)).await {
        Ok(running) => {
            info!("MCP server initialized, waiting for transport to close");
            // Wait until the transport is closed or the service is cancelled.
            running.waiting().await;
            info!("MCP server transport closed cleanly");
        }
        Err(e) => {
            error!(error = %e, "MCP server failed");
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::create_command_channel;
    use crate::config::AppConfig;

    /// Verify get_info() returns the expected server name.
    #[test]
    fn test_get_info_server_name() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, _receiver) = create_command_channel(4);
        let server = McpBotServer::new(state, sender);

        let info = server.get_info();
        assert_eq!(info.server_info.name, "minecraft-mcp-rs");
    }

    /// Verify get_info() returns the Cargo package version.
    #[test]
    fn test_get_info_version() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, _receiver) = create_command_channel(4);
        let server = McpBotServer::new(state, sender);

        let info = server.get_info();
        assert_eq!(info.server_info.version, env!("CARGO_PKG_VERSION"));
    }

    /// Verify get_info() has tools enabled in capabilities.
    #[test]
    fn test_get_info_tools_enabled() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, _receiver) = create_command_channel(4);
        let server = McpBotServer::new(state, sender);

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
        let (sender, _receiver) = create_command_channel(4);
        let server = McpBotServer::new(state, sender);

        let info = server.get_info();
        assert!(info.instructions.is_some());
        assert!(info.instructions.unwrap().contains("Minecraft"));
    }

    /// Movement tool integration tests — verify offline rejection.
    #[tokio::test]
    async fn test_movement_tools_offline() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, _receiver) = create_command_channel(4);
        let server = McpBotServer::new(state, sender);

        let result = server.move_to(Parameters(MoveToInput { x: 0, y: 64, z: 0 })).await;
        assert!(result.contains("not connected"));

        let result = server.walk_direction(Parameters(WalkDirectionInput { direction: "north".into(), distance: 1 })).await;
        assert!(result.contains("not connected"));

        let result = server.jump(Parameters(JumpInput {})).await;
        assert!(result.contains("not connected"));

        let result = server.teleport(Parameters(TeleportInput { x: 0, y: 64, z: 0 })).await;
        assert!(result.contains("requires Creative") || result.contains("not connected"));
    }

    /// Container tool integration tests — verify offline/no-container rejection.
    #[tokio::test]
    async fn test_container_tools_offline() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, _receiver) = create_command_channel(4);
        let server = McpBotServer::new(state, sender);

        let result = server.open_container(Parameters(OpenContainerInput { x: 0, y: 64, z: 0 })).await;
        assert!(result.contains("not connected"));

        let result = server.take_from_container(Parameters(TakeFromContainerInput { slot: 0, count: Some(1) })).await;
        assert!(result.contains("No container is currently open"));

        let result = server.put_into_container(Parameters(PutIntoContainerInput { slot: 0, count: Some(1) })).await;
        assert!(result.contains("No container is currently open"));

        let result = server.close_container(Parameters(CloseContainerInput {})).await;
        assert!(result.contains("No container is currently open"));
    }

    /// Combat tool integration tests — verify offline/entity-not-found rejection.
    #[tokio::test]
    async fn test_combat_tools_offline() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, _receiver) = create_command_channel(4);
        let server = McpBotServer::new(state, sender);

        let result = server.attack_entity(Parameters(AttackEntityInput { entity_id: 42 })).await;
        assert!(result.contains("not found") || result.contains("not connected"));

        let result = server.shield_block(Parameters(ShieldBlockInput { blocking: true })).await;
        assert!(result.contains("not connected"));
    }

    /// Query tools return offline error when the bot is not connected.
    #[tokio::test]
    async fn test_query_tools_offline() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, _receiver) = create_command_channel(4);
        let server = McpBotServer::new(state, sender);

        let offline = r#"{"error":"Bot is currently offline"}"#;
        assert_eq!(server.get_self_info().await, offline);
        assert_eq!(server.get_inventory().await, offline);
        assert_eq!(server.get_nearby_blocks().await, offline);
        assert_eq!(server.get_nearby_entities().await, offline);
        assert_eq!(server.get_chunk_summary().await, offline);
    }

    /// is_connected returns false when bot is offline.
    #[tokio::test]
    async fn test_is_connected_offline() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, _receiver) = create_command_channel(4);
        let server = McpBotServer::new(state, sender);

        assert_eq!(server.is_connected().await, r#"{"connected":false}"#);
    }

    /// is_connected returns true when bot is online.
    #[tokio::test]
    async fn test_is_connected_online() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        state.set_online(true);
        let (sender, _receiver) = create_command_channel(4);
        let server = McpBotServer::new(state, sender);

        assert_eq!(server.is_connected().await, r#"{"connected":true}"#);
    }

    // ── Block tool integration tests ───────────────────────────────────

    #[tokio::test]
    async fn test_break_block_offline_via_server() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, _receiver) = create_command_channel(4);
        let server = McpBotServer::new(state, sender);

        let result = server
            .break_block(Parameters(BreakBlockInput {
                x: 0,
                y: 64,
                z: 0,
                use_best_tool: None,
            }))
            .await;
        assert!(result.contains("not connected"));
    }

    #[tokio::test]
    async fn test_place_block_offline_via_server() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, _receiver) = create_command_channel(4);
        let server = McpBotServer::new(state, sender);

        let result = server
            .place_block(Parameters(PlaceBlockInput {
                x: 0,
                y: 64,
                z: 0,
                item_slot: 1,
            }))
            .await;
        assert!(result.contains("not connected"));
    }

    #[tokio::test]
    async fn test_use_item_on_block_offline_via_server() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, _receiver) = create_command_channel(4);
        let server = McpBotServer::new(state, sender);

        let result = server
            .use_item_on_block(Parameters(UseItemOnBlockInput {
                x: 0,
                y: 64,
                z: 0,
                item_slot: None,
            }))
            .await;
        assert!(result.contains("not connected"));
    }

    #[tokio::test]
    async fn test_break_block_invalid_coords_via_server() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        state.set_online(true);
        let (sender, _receiver) = create_command_channel(4);
        let server = McpBotServer::new(state, sender);

        let result = server
            .break_block(Parameters(BreakBlockInput {
                x: 0,
                y: 500,
                z: 0,
                use_best_tool: None,
            }))
            .await;
        assert!(result.contains("out of bounds") || result.contains("out of range"));
    }

    #[tokio::test]
    async fn test_place_block_invalid_slot_via_server() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        state.set_online(true);
        let (sender, _receiver) = create_command_channel(4);
        let server = McpBotServer::new(state, sender);

        let result = server
            .place_block(Parameters(PlaceBlockInput {
                x: 0,
                y: 64,
                z: 0,
                item_slot: 9,
            }))
            .await;
        assert!(result.contains("must be 0-8"));
    }

    #[tokio::test]
    async fn test_use_item_on_block_invalid_slot_via_server() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        state.set_online(true);
        let (sender, _receiver) = create_command_channel(4);
        let server = McpBotServer::new(state, sender);

        let result = server
            .use_item_on_block(Parameters(UseItemOnBlockInput {
                x: 0,
                y: 64,
                z: 0,
                item_slot: Some(10),
            }))
            .await;
        assert!(result.contains("must be 0-8"));
    }

    // ── Item tool integration tests ─────────────────────────────────────

    #[tokio::test]
    async fn test_switch_hotbar_slot_offline_via_server() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, _receiver) = create_command_channel(4);
        let server = McpBotServer::new(state, sender);

        let result = server
            .switch_hotbar_slot(Parameters(SwitchHotbarSlotInput { slot: 0 }))
            .await;
        assert!(result.contains("not connected"));
    }

    #[tokio::test]
    async fn test_drop_item_offline_via_server() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, _receiver) = create_command_channel(4);
        let server = McpBotServer::new(state, sender);

        let result = server
            .drop_item(Parameters(DropItemInput {
                slot: 0,
                count: Some(1),
            }))
            .await;
        assert!(result.contains("not connected"));
    }

    #[tokio::test]
    async fn test_use_item_offline_via_server() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, _receiver) = create_command_channel(4);
        let server = McpBotServer::new(state, sender);

        let result = server
            .use_item(Parameters(UseItemInput { item_slot: None }))
            .await;
        assert!(result.contains("not connected"));
    }

    #[tokio::test]
    async fn test_equip_tool_offline_via_server() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        let (sender, _receiver) = create_command_channel(4);
        let server = McpBotServer::new(state, sender);

        let result = server
            .equip_tool(Parameters(EquipToolInput {
                tool_type: "pickaxe".into(),
                material_preference: None,
            }))
            .await;
        assert!(result.contains("not connected"));
    }

    #[tokio::test]
    async fn test_switch_hotbar_slot_invalid_via_server() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        state.set_online(true);
        let (sender, _receiver) = create_command_channel(4);
        let server = McpBotServer::new(state, sender);

        let result = server
            .switch_hotbar_slot(Parameters(SwitchHotbarSlotInput { slot: 9 }))
            .await;
        assert!(result.contains("must be 0-8"));
    }

    #[tokio::test]
    async fn test_equip_tool_unknown_type_via_server() {
        let state = Arc::new(SharedState::new(AppConfig::default()));
        state.set_online(true);
        let (sender, _receiver) = create_command_channel(4);
        let server = McpBotServer::new(state, sender);

        let result = server
            .equip_tool(Parameters(EquipToolInput {
                tool_type: "hoe".into(),
                material_preference: None,
            }))
            .await;
        assert!(result.contains("Unknown tool type"));
    }
}
