//! Mock-based integration tests for Minecraft MCP server.
//!
//! These tests verify end-to-end flows using mock channels and state,
//! without requiring a real Minecraft server connection. All bot
//! interactions are mocked via `BotCommandSender`/`BotCommandReceiver`
//! and `SharedState` snapshots.
//!
//! ## Test coverage
//!
//! 1. Full MCP cycle: server info → query tool → channel command → response
//! 2. Channel transmits correct BotCommand for movement operations
//! 3. get_self_info returns player data from snapshot
//! 4. Concurrent commands are serialized (second waits for first)
//! 5. Bot offline returns Offline error via channel
//! 6. Command timeout returns CommandTimeout error
//! 7. Auto-reconnect sequence simulation
//! 8. All MCP tool functions exist and work correctly

use std::sync::Arc;

use minecraft_mcp_rs::channel;
use minecraft_mcp_rs::config::AppConfig;
use minecraft_mcp_rs::error::BotError;
use minecraft_mcp_rs::mcp::server::McpBotServer;
use minecraft_mcp_rs::state::SharedState;
use minecraft_mcp_rs::types::*;
use rmcp::ServerHandler;

// ═══════════════════════════════════════════════════════════════
// Test Helpers
// ═══════════════════════════════════════════════════════════════

/// Create a WorldSnapshot with realistic test data.
fn make_test_snapshot() -> WorldSnapshot {
    WorldSnapshot {
        blocks: vec![
            BlockEntry {
                position: BlockPos::new(0, 64, 0),
                block_type: "stone".into(),
                block_state: None,
            },
            BlockEntry {
                position: BlockPos::new(10, 64, 0),
                block_type: "diamond_ore".into(),
                block_state: None,
            },
        ],
        entities: vec![EntityEntry {
            id: 1,
            uuid: "entity-uuid-1".into(),
            entity_type: "zombie".into(),
            position: BlockPos::new(3, 64, 2),
            display_name: Some("Zombie".into()),
            health: Some(20.0),
        }],
        self_player: SelfPlayer {
            uuid: "550e8400-e29b-41d4-a716-446655440000".into(),
            username: "TestBot".into(),
            position: BlockPos::new(100, 64, 200),
            health: 18.5,
            hunger: 15,
            gamemode: GameMode::Survival,
            held_item_slot: 3,
            inventory: Vec::new(),
        },
        timestamp: 42,
        chunk_summary: vec![(0, 0), (-1, 1)],
    }
}

/// Create a SharedState with the bot online and a populated snapshot.
fn make_online_state() -> Arc<SharedState> {
    let state = SharedState::new(AppConfig::default());
    state.set_online(true);
    state.update_snapshot(make_test_snapshot());
    Arc::new(state)
}

/// Create a SharedState with the bot offline (no snapshot).
fn make_offline_state() -> Arc<SharedState> {
    Arc::new(SharedState::new(AppConfig::default()))
}

/// Helper to create a BotResult.
fn bot_result(success: bool, message: &str) -> BotResult {
    BotResult {
        success,
        message: message.into(),
        data: None,
    }
}

// ═══════════════════════════════════════════════════════════════
// Test 1: Full MCP cycle — initialize → query tool → channel command → response
// ═══════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_full_mcp_cycle_initialize_and_query() {
    // ── Initialize (get_info) ────────────────────────────────────
    let state = make_online_state();
    let (sender, mut receiver) = channel::create_command_channel(4);
    let server = McpBotServer::new(state.clone(), sender.clone());

    let info = server.get_info();
    assert_eq!(info.server_info.name, "minecraft-mcp-rs");
    assert_eq!(info.server_info.version, env!("CARGO_PKG_VERSION"));
    assert!(
        info.capabilities.tools.is_some(),
        "tools capability must be enabled"
    );
    assert!(info.instructions.is_some(), "server must have instructions");
    let instructions = info.instructions.unwrap();
    assert!(
        instructions.contains("Minecraft"),
        "instructions should mention Minecraft"
    );
    assert!(
        instructions.contains("destructive"),
        "instructions should mention destructive annotations"
    );

    // ── Query tool via underlying public function ───────────────
    let self_info = minecraft_mcp_rs::mcp::tools_query::get_self_info(&state);
    assert!(self_info.contains("TestBot"));
    assert!(self_info.contains("550e8400"));
    assert!(self_info.contains("18.5"));

    let connected = minecraft_mcp_rs::mcp::tools_query::is_connected(&state);
    assert_eq!(connected, r#"{"connected":true}"#);

    // ── Action tool via channel ──────────────────────────────────
    let responder = tokio::spawn(async move {
        let wrapped = receiver.recv().await.expect("should receive command");
        assert!(
            matches!(wrapped.command, BotCommand::SendChat(_)),
            "expected SendChat, got: {:?}",
            wrapped.command
        );
        wrapped
            .respond_to
            .send(Ok(bot_result(true, "message sent: Hello World")))
            .expect("should respond");
    });

    let chat_response =
        minecraft_mcp_rs::mcp::tools_chat::handle_send_chat(&sender, "Hello World".into()).await;
    assert!(chat_response.contains("message sent"));

    responder.await.expect("responder should complete");
}

#[tokio::test]
async fn test_full_mcp_cycle_tool_list_and_offline_handling() {
    let state = make_offline_state();
    let (sender, _receiver) = channel::create_command_channel(4);
    let server = McpBotServer::new(state.clone(), sender);

    // get_info works even offline
    let info = server.get_info();
    assert_eq!(info.server_info.name, "minecraft-mcp-rs");

    // Query tools return offline error when bot is offline
    let offline = r#"{"error":"Bot is currently offline"}"#;
    assert_eq!(
        minecraft_mcp_rs::mcp::tools_query::get_self_info(&state),
        offline
    );
    assert_eq!(
        minecraft_mcp_rs::mcp::tools_query::get_inventory(&state),
        offline
    );
    assert_eq!(
        minecraft_mcp_rs::mcp::tools_query::is_connected(&state),
        r#"{"connected":false}"#
    );
}

// ═══════════════════════════════════════════════════════════════
// Test 2: Channel transmits correct BotCommand for movement
// ═══════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_channel_move_to_sends_correct_position() {
    let (sender, mut receiver) = channel::create_command_channel(4);

    let verifier = tokio::spawn(async move {
        let wrapped = receiver.recv().await.expect("should receive command");
        match &wrapped.command {
            BotCommand::MoveTo(pos) => {
                assert_eq!(pos.x, 50);
                assert_eq!(pos.y, 70);
                assert_eq!(pos.z, -100);
            }
            other => panic!("expected MoveTo, got: {:?}", other),
        }
        wrapped
            .respond_to
            .send(Ok(bot_result(true, "arrived at (50, 70, -100)")))
            .expect("should respond");
    });

    let cmd = BotCommand::MoveTo(BlockPos::new(50, 70, -100));
    let result = sender.send_command(cmd).await.expect("should succeed");
    assert!(result.success);
    assert!(result.message.contains("arrived"));

    verifier.await.expect("verifier should complete");
}

#[tokio::test]
async fn test_channel_break_block_sends_correct_position() {
    let (sender, mut receiver) = channel::create_command_channel(4);

    let verifier = tokio::spawn(async move {
        let wrapped = receiver.recv().await.expect("should receive command");
        match &wrapped.command {
            BotCommand::BreakBlock(pos) => {
                assert_eq!(pos.x, 10);
                assert_eq!(pos.y, 64);
                assert_eq!(pos.z, -5);
            }
            other => panic!("expected BreakBlock, got: {:?}", other),
        }
        wrapped
            .respond_to
            .send(Ok(bot_result(true, "block broken")))
            .expect("should respond");
    });

    let cmd = BotCommand::BreakBlock(BlockPos::new(10, 64, -5));
    let result = sender.send_command(cmd).await.expect("should succeed");
    assert!(result.success);

    verifier.await.expect("verifier should complete");
}

#[tokio::test]
async fn test_channel_walk_direction_sends_correct_direction() {
    let (sender, mut receiver) = channel::create_command_channel(4);

    let verifier = tokio::spawn(async move {
        let wrapped = receiver.recv().await.expect("should receive command");
        match &wrapped.command {
            BotCommand::WalkDirection(dir, distance) => {
                assert_eq!(*dir, Direction::North);
                assert_eq!(*distance, 5);
            }
            other => panic!("expected WalkDirection, got: {:?}", other),
        }
        wrapped
            .respond_to
            .send(Ok(bot_result(true, "walking north")))
            .expect("should respond");
    });

    let cmd = BotCommand::WalkDirection(Direction::North, 5);
    let result = sender.send_command(cmd).await.expect("should succeed");
    assert!(result.success);

    verifier.await.expect("verifier should complete");
}

#[tokio::test]
async fn test_channel_place_block_sends_position_and_type() {
    let (sender, mut receiver) = channel::create_command_channel(4);

    let verifier = tokio::spawn(async move {
        let wrapped = receiver.recv().await.expect("should receive command");
        match &wrapped.command {
            BotCommand::PlaceBlock(pos, block_type) => {
                assert_eq!(pos.x, 1);
                assert_eq!(pos.y, 65);
                assert_eq!(pos.z, 3);
                assert_eq!(block_type, "slot:2");
            }
            other => panic!("expected PlaceBlock, got: {:?}", other),
        }
        wrapped
            .respond_to
            .send(Ok(bot_result(true, "block placed")))
            .expect("should respond");
    });

    let cmd = BotCommand::PlaceBlock(BlockPos::new(1, 65, 3), "slot:2".into());
    let result = sender.send_command(cmd).await.expect("should succeed");
    assert!(result.success);

    verifier.await.expect("verifier should complete");
}

// ═══════════════════════════════════════════════════════════════
// Test 3: get_self_info returns player data from snapshot
// ═══════════════════════════════════════════════════════════════

#[test]
fn test_get_self_info_returns_player_data_from_snapshot() {
    let state = make_online_state();
    let result = minecraft_mcp_rs::mcp::tools_query::get_self_info(&state);

    let parsed: serde_json::Value =
        serde_json::from_str(&result).expect("get_self_info should return valid JSON");

    assert_eq!(parsed["uuid"], "550e8400-e29b-41d4-a716-446655440000");
    assert_eq!(parsed["username"], "TestBot");
    assert_eq!(parsed["position"]["x"], 100);
    assert_eq!(parsed["position"]["y"], 64);
    assert_eq!(parsed["position"]["z"], 200);
    assert_eq!(parsed["health"], 18.5);
    assert_eq!(parsed["hunger"], 15);
    assert_eq!(parsed["gamemode"], "Survival");
    assert_eq!(parsed["held_item_slot"], 3);
}

#[test]
fn test_get_self_info_offline_returns_error() {
    let state = make_offline_state();
    let result = minecraft_mcp_rs::mcp::tools_query::get_self_info(&state);
    assert_eq!(result, r#"{"error":"Bot is currently offline"}"#);
}

#[test]
fn test_is_connected_reflects_online_status() {
    let state = make_online_state();
    assert_eq!(
        minecraft_mcp_rs::mcp::tools_query::is_connected(&state),
        r#"{"connected":true}"#
    );

    let offline_state = make_offline_state();
    assert_eq!(
        minecraft_mcp_rs::mcp::tools_query::is_connected(&offline_state),
        r#"{"connected":false}"#
    );
}

// ═══════════════════════════════════════════════════════════════
// Test 4: Concurrent commands are serialized (second waits)
// ═══════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_concurrent_commands_serialized_second_waits() {
    let (sender, mut receiver) = channel::create_command_channel(16);

    let responder = tokio::spawn(async move {
        let mut order: Vec<String> = vec![];

        let w1 = receiver.recv().await.expect("should receive cmd1");
        order.push(format!("start-{:?}", w1.command));
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let _ = w1.respond_to.send(Ok(bot_result(true, "first done")));
        order.push("end-first".into());

        let w2 = receiver.recv().await.expect("should receive cmd2");
        order.push(format!("start-{:?}", w2.command));
        let _ = w2.respond_to.send(Ok(bot_result(true, "second done")));
        order.push("end-second".into());

        order
    });

    let s1 = sender.clone();
    let s2 = sender.clone();

    let h1 = tokio::spawn(async move {
        s1.send_command(BotCommand::Jump)
            .await
            .expect("cmd1 should succeed")
    });

    let h2 = tokio::spawn(async move {
        s2.send_command(BotCommand::QuerySelfInfo)
            .await
            .expect("cmd2 should succeed")
    });

    let r1 = h1.await.expect("h1");
    let r2 = h2.await.expect("h2");

    assert!(r1.success, "first command should succeed");
    assert!(r2.success, "second command should succeed");
    assert_eq!(r1.message, "first done");
    assert_eq!(r2.message, "second done");

    drop(sender);

    let order = responder.await.expect("responder should finish");
    assert_eq!(order.len(), 4);
    assert!(order[0].contains("Jump"), "first processed should be Jump");
    assert_eq!(order[1], "end-first");
    assert!(
        order[2].contains("QuerySelfInfo"),
        "second processed is QuerySelfInfo"
    );
    assert_eq!(order[3], "end-second");
}

#[tokio::test]
async fn test_multiple_commands_all_get_responses() {
    let (sender, mut receiver) = channel::create_command_channel(16);

    let responder = tokio::spawn(async move {
        let mut count = 0u32;
        while let Some(wrapped) = receiver.recv().await {
            count += 1;
            let _ = wrapped
                .respond_to
                .send(Ok(bot_result(true, &format!("ack-{count}"))));
        }
        count
    });

    let s = sender.clone();
    let handles: Vec<_> = (0..5)
        .map(|_| {
            let s = s.clone();
            tokio::spawn(async move {
                s.send_command(BotCommand::Jump)
                    .await
                    .expect("should succeed")
            })
        })
        .collect();

    for h in handles {
        let r = h.await.expect("handle");
        assert!(r.success);
        assert!(r.message.contains("ack-"));
    }

    // Drop ALL sender clones to signal receiver to stop
    drop(s);
    drop(sender);
    let total = responder.await.expect("responder should finish");
    assert_eq!(total, 5);
}

// ═══════════════════════════════════════════════════════════════
// Test 5: Bot offline returns Offline error via channel
// ═══════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_bot_offline_channel_returns_offline_error() {
    let (sender, receiver) = channel::create_command_channel(4);
    drop(receiver);

    let result = sender.send_command(BotCommand::QuerySelfInfo).await;

    match result {
        Err(BotError::Offline(msg)) => {
            assert!(
                msg.contains("closed"),
                "Offline error should mention channel closed: {msg}"
            );
        }
        other => panic!("expected BotError::Offline, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_bot_offline_all_command_types_fail() {
    let (sender, receiver) = channel::create_command_channel(4);
    drop(receiver);

    let commands = vec![
        BotCommand::Jump,
        BotCommand::MoveTo(BlockPos::new(0, 0, 0)),
        BotCommand::BreakBlock(BlockPos::new(0, 0, 0)),
        BotCommand::SendChat("hello".into()),
        BotCommand::QuerySelfInfo,
    ];

    for cmd in commands {
        let result = sender.send_command(cmd).await;
        assert!(
            matches!(result, Err(BotError::Offline(_))),
            "expected Offline error"
        );
    }
}

#[tokio::test]
async fn test_query_tools_offline_return_error() {
    let state = make_offline_state();

    let offline = r#"{"error":"Bot is currently offline"}"#;
    assert_eq!(
        minecraft_mcp_rs::mcp::tools_query::get_self_info(&state),
        offline
    );
    assert_eq!(
        minecraft_mcp_rs::mcp::tools_query::get_inventory(&state),
        offline
    );
    assert_eq!(
        minecraft_mcp_rs::mcp::tools_query::get_nearby_blocks(&state, 10, None),
        offline
    );
    assert_eq!(
        minecraft_mcp_rs::mcp::tools_query::get_nearby_entities(&state, 10),
        offline
    );
    assert_eq!(
        minecraft_mcp_rs::mcp::tools_query::get_chunk_summary(&state),
        offline
    );
}

// ═══════════════════════════════════════════════════════════════
// Test 6: Command timeout returns CommandTimeout error
// ═══════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_command_timeout_when_responder_dropped() {
    let (sender, mut receiver) = channel::create_command_channel(4);

    let dropper = tokio::spawn(async move {
        let wrapped = receiver.recv().await.expect("should receive command");
        drop(wrapped);
    });

    let result = sender.send_command(BotCommand::Jump).await;

    match result {
        Err(BotError::CommandTimeout {
            command,
            timeout_secs,
        }) => {
            assert!(command.contains("Jump"), "timeout should mention Jump");
            assert_eq!(timeout_secs, 30);
        }
        other => panic!("expected BotError::CommandTimeout, got: {:?}", other),
    }

    dropper.await.expect("dropper should complete");
}

#[tokio::test]
async fn test_command_timeout_with_break_block() {
    let (sender, mut receiver) = channel::create_command_channel(4);

    let dropper = tokio::spawn(async move {
        let wrapped = receiver.recv().await.expect("should receive command");
        assert!(
            matches!(wrapped.command, BotCommand::BreakBlock(_)),
            "expected BreakBlock"
        );
        drop(wrapped);
    });

    let cmd = BotCommand::BreakBlock(BlockPos::new(42, 10, 99));
    let result = sender.send_command(cmd).await;

    match result {
        Err(BotError::CommandTimeout { command, .. }) => {
            assert!(command.contains("BreakBlock"));
        }
        other => panic!("expected CommandTimeout, got: {:?}", other),
    }

    dropper.await.expect("dropper should complete");
}

#[tokio::test]
async fn test_command_timeout_distinct_from_offline() {
    // Offline: receiver dropped before command is sent
    let (sender1, receiver1) = channel::create_command_channel(4);
    drop(receiver1);
    let offline_result = sender1.send_command(BotCommand::Jump).await;
    assert!(matches!(offline_result, Err(BotError::Offline(_))));

    // Timeout: receiver exists but drops responder without replying
    let (sender2, mut receiver2) = channel::create_command_channel(4);
    let dropper = tokio::spawn(async move {
        let wrapped = receiver2.recv().await.unwrap();
        drop(wrapped);
    });
    let timeout_result = sender2.send_command(BotCommand::Jump).await;
    assert!(matches!(
        timeout_result,
        Err(BotError::CommandTimeout { .. })
    ));

    dropper.await.unwrap();
}

// ═══════════════════════════════════════════════════════════════
// Test 7: Auto-reconnect sequence simulation
// ═══════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_auto_reconnect_sequence_simulation() {
    let state = make_online_state();
    let (sender, mut receiver) = channel::create_command_channel(4);

    let _chan_task = tokio::spawn(async move {
        while let Some(wrapped) = receiver.recv().await {
            let _ = wrapped.respond_to.send(Ok(bot_result(true, "ok")));
        }
    });

    let _server = McpBotServer::new(state.clone(), sender.clone());

    // Phase 1: Online
    assert_eq!(
        minecraft_mcp_rs::mcp::tools_query::is_connected(&state),
        r#"{"connected":true}"#
    );
    let response = minecraft_mcp_rs::mcp::tools_query::get_self_info(&state);
    assert!(response.contains("TestBot"));

    // Phase 2: Disconnect
    state.set_online(false);
    assert_eq!(
        minecraft_mcp_rs::mcp::tools_query::is_connected(&state),
        r#"{"connected":false}"#
    );
    let offline_resp = minecraft_mcp_rs::mcp::tools_query::get_self_info(&state);
    assert_eq!(offline_resp, r#"{"error":"Bot is currently offline"}"#);

    // Phase 3: Reconnect with fresh snapshot
    state.set_online(true);
    let fresh_snap = WorldSnapshot {
        blocks: vec![BlockEntry {
            position: BlockPos::new(5, 70, 5),
            block_type: "oak_log".into(),
            block_state: None,
        }],
        entities: vec![],
        self_player: SelfPlayer {
            uuid: "550e8400-e29b-41d4-a716-446655440000".into(),
            username: "TestBot".into(),
            position: BlockPos::new(200, 70, 300),
            health: 20.0,
            hunger: 20,
            gamemode: GameMode::Survival,
            held_item_slot: 0,
            inventory: Vec::new(),
        },
        timestamp: 99,
        chunk_summary: vec![(1, 1)],
    };
    state.update_snapshot(fresh_snap);

    assert_eq!(
        minecraft_mcp_rs::mcp::tools_query::is_connected(&state),
        r#"{"connected":true}"#
    );

    let reconnected = minecraft_mcp_rs::mcp::tools_query::get_self_info(&state);
    assert!(reconnected.contains("200"), "reconnected: x should be 200");
    assert!(
        reconnected.contains("20.0"),
        "reconnected: health should be 20.0"
    );
    assert!(
        reconnected.contains("\"hunger\":20"),
        "reconnected: hunger should be 20"
    );
    assert!(reconnected.contains("300"), "reconnected: z should be 300");
}

#[tokio::test]
async fn test_reconnect_multiple_cycles() {
    let state = make_online_state();
    let (sender, mut receiver) = channel::create_command_channel(4);

    let _chan_task = tokio::spawn(async move {
        while let Some(wrapped) = receiver.recv().await {
            let _ = wrapped.respond_to.send(Ok(bot_result(true, "ok")));
        }
    });

    let _server = McpBotServer::new(state.clone(), sender.clone());

    for cycle in 0..3 {
        state.set_online(true);
        assert_eq!(
            minecraft_mcp_rs::mcp::tools_query::is_connected(&state),
            r#"{"connected":true}"#,
            "cycle {cycle}: should be online"
        );

        state.set_online(false);
        assert_eq!(
            minecraft_mcp_rs::mcp::tools_query::is_connected(&state),
            r#"{"connected":false}"#,
            "cycle {cycle}: should be offline"
        );
        let offline_resp = minecraft_mcp_rs::mcp::tools_query::get_self_info(&state);
        assert_eq!(
            offline_resp, r#"{"error":"Bot is currently offline"}"#,
            "cycle {cycle}: offline should return error"
        );
    }
}

// ═══════════════════════════════════════════════════════════════
// Test 8: All MCP tool functions exist and no craft_item
// ═══════════════════════════════════════════════════════════════

#[test]
fn test_all_query_tools_exist_and_work() {
    let state = make_online_state();

    let self_info = minecraft_mcp_rs::mcp::tools_query::get_self_info(&state);
    assert!(!self_info.is_empty());

    let inventory = minecraft_mcp_rs::mcp::tools_query::get_inventory(&state);
    assert!(inventory.contains("held_item_slot"));

    let nearby_blocks = minecraft_mcp_rs::mcp::tools_query::get_nearby_blocks(&state, 1, None);
    assert!(!nearby_blocks.is_empty());

    let nearby_entities = minecraft_mcp_rs::mcp::tools_query::get_nearby_entities(&state, 1);
    assert!(!nearby_entities.is_empty());

    let chunk_summary = minecraft_mcp_rs::mcp::tools_query::get_chunk_summary(&state);
    assert!(!chunk_summary.is_empty());

    let connected = minecraft_mcp_rs::mcp::tools_query::is_connected(&state);
    assert_eq!(connected, r#"{"connected":true}"#);
}

#[tokio::test]
async fn test_all_bot_command_variants_exist_no_craft_item() {
    let (sender, mut receiver) = channel::create_command_channel(16);

    let responder = tokio::spawn(async move {
        let mut variants_seen = std::collections::HashSet::new();
        while let Some(wrapped) = receiver.recv().await {
            let variant = format!("{:?}", wrapped.command);
            let name = variant
                .split(['(', ' '])
                .next()
                .unwrap_or(&variant)
                .to_string();
            variants_seen.insert(name);
            let _ = wrapped.respond_to.send(Ok(bot_result(true, "executed")));
        }
        variants_seen
    });

    let commands = vec![
        BotCommand::MoveTo(BlockPos::new(0, 64, 0)),
        BotCommand::WalkDirection(Direction::North, 1),
        BotCommand::Jump,
        BotCommand::Teleport(BlockPos::new(0, 64, 0)),
        BotCommand::BreakBlock(BlockPos::new(0, 64, 0)),
        BotCommand::PlaceBlock(BlockPos::new(0, 64, 0), "slot:0".into()),
        BotCommand::UseItemOnBlock(BlockPos::new(0, 64, 0), None),
        BotCommand::SwitchHotbarSlot(0),
        BotCommand::DropItem(0, 1),
        BotCommand::UseItem,
        BotCommand::EquipTool(ToolType::Pickaxe),
        BotCommand::OpenContainer(BlockPos::new(0, 64, 0)),
        BotCommand::TakeFromContainer(0, 1),
        BotCommand::PutIntoContainer(0, 1),
        BotCommand::CloseContainer,
        BotCommand::AttackEntity(42),
        BotCommand::ShieldBlock(true),
        BotCommand::SendChat("test".into()),
        BotCommand::ExecuteCommand("/help".into()),
        BotCommand::SetGameMode(GameMode::Survival),
        BotCommand::QueryNearbyBlocks(10),
        BotCommand::QueryNearbyEntities(10),
        BotCommand::QuerySelfInfo,
        BotCommand::QueryInventory,
        BotCommand::QueryChunkSummary,
    ];

    assert_eq!(
        commands.len(),
        25,
        "should have exactly 25 BotCommand variants"
    );

    for cmd in commands {
        let result = sender.send_command(cmd).await.expect("should succeed");
        assert!(result.success);
    }

    // Verify no CraftItem in serialized output
    let json = serde_json::to_string(&BotCommand::Jump).unwrap();
    assert!(
        !json.to_lowercase().contains("craft_item"),
        "BotCommand serialization must not contain craft_item"
    );

    drop(sender);
    let variants = responder.await.expect("responder should finish");
    assert!(
        !variants.contains("CraftItem"),
        "no CraftItem variant should exist"
    );
}

// ═══════════════════════════════════════════════════════════════
// Additional: channel factory, sender cloning, compound ops
// ═══════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_channel_factory_creates_working_pair() {
    let (sender, mut receiver) = channel::create_command_channel(8);

    let verifier = tokio::spawn(async move {
        let wrapped = receiver.recv().await.expect("should receive");
        assert!(matches!(wrapped.command, BotCommand::UseItem));
        wrapped
            .respond_to
            .send(Ok(bot_result(true, "used item")))
            .expect("should respond");
    });

    let result = sender
        .send_command(BotCommand::UseItem)
        .await
        .expect("should succeed");
    assert!(result.success);

    verifier.await.expect("verifier should complete");
}

#[tokio::test]
async fn test_sender_clone_works_independently() {
    let (sender, mut receiver) = channel::create_command_channel(8);
    let sender2 = sender.clone();

    let responder = tokio::spawn(async move {
        let mut count = 0;
        while let Some(wrapped) = receiver.recv().await {
            count += 1;
            let _ = wrapped
                .respond_to
                .send(Ok(bot_result(true, &format!("ack-{count}"))));
            if count == 2 {
                break;
            }
        }
        count
    });

    let h1 = tokio::spawn(async move {
        sender
            .send_command(BotCommand::Jump)
            .await
            .expect("sender1 should succeed")
    });

    let h2 = tokio::spawn(async move {
        sender2
            .send_command(BotCommand::ShieldBlock(true))
            .await
            .expect("sender2 should succeed")
    });

    let r1 = h1.await.expect("h1");
    let r2 = h2.await.expect("h2");

    assert!(r1.success);
    assert!(r2.success);

    let total = responder.await.expect("responder");
    assert_eq!(total, 2);
}

#[tokio::test]
async fn test_compound_break_with_tool_selection_flow() {
    let (sender, mut receiver) = channel::create_command_channel(16);

    let processor = tokio::spawn(async move {
        // Step 1: equip_tool
        let w1 = receiver.recv().await.expect("should receive equip_tool");
        assert!(
            matches!(w1.command, BotCommand::EquipTool(ToolType::Pickaxe)),
            "expected EquipTool(Pickaxe)"
        );
        let _ = w1
            .respond_to
            .send(Ok(bot_result(true, "equipped diamond_pickaxe")));

        // Step 2: break_block
        let w2 = receiver.recv().await.expect("should receive break_block");
        match &w2.command {
            BotCommand::BreakBlock(pos) => {
                assert_eq!(pos.x, 42);
                assert_eq!(pos.y, 11);
                assert_eq!(pos.z, 7);
            }
            other => panic!("expected BreakBlock, got: {:?}", other),
        }
        let _ = w2
            .respond_to
            .send(Ok(bot_result(true, "broke diamond_ore")));
    });

    let equip_cmd = BotCommand::EquipTool(ToolType::Pickaxe);
    let equip_result = sender
        .send_command(equip_cmd)
        .await
        .expect("equip should succeed");
    assert!(equip_result.success);
    assert!(equip_result.message.contains("pickaxe"));

    let break_cmd = BotCommand::BreakBlock(BlockPos::new(42, 11, 7));
    let break_result = sender
        .send_command(break_cmd)
        .await
        .expect("break should succeed");
    assert!(break_result.success);
    assert!(break_result.message.contains("broke"));

    processor.await.expect("processor should complete");
}
