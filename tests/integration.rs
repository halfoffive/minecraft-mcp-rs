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
            position_precise: None,
            yaw: None,
        },
        timestamp: 42,
        chunk_summary: vec![(0, 0), (-1, 1)],
        commands_enabled: None,
        ..Default::default()
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

/// Create a minimal `Arc<SharedState>` for tests that just need a state
/// to feed to [`channel::create_command_channel`] without caring about
/// online status or snapshot content.
fn make_test_state() -> Arc<SharedState> {
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
    let (sender, mut receiver) = channel::create_command_channel(4, state.clone());
    // Empty slot: the receiver is leased by the responder below, not by
    // connect_bot — the server only needs the slot to exist.
    let empty_slot: Arc<std::sync::Mutex<Option<channel::BotCommandReceiver>>> =
        Arc::new(std::sync::Mutex::new(None));
    let server = McpBotServer::new(state.clone(), sender.clone(), empty_slot);

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
    let self_info = minecraft_mcp_rs::mcp::tools_query::get_self_info(
        &state,
        minecraft_mcp_rs::mcp::tools_query::SelfInfoInput { force: false },
    )
    .await
    .unwrap();
    assert!(self_info.contains("TestBot"));
    assert!(self_info.contains("550e8400"));
    assert!(self_info.contains("18.5"));

    let connected = minecraft_mcp_rs::mcp::tools_query::is_connected(&state).unwrap();
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
        minecraft_mcp_rs::mcp::tools_chat::handle_send_chat(&state, &sender, "Hello World".into())
            .await
            .unwrap();
    assert!(chat_response.contains("message sent"));

    responder.await.expect("responder should complete");
}

#[tokio::test]
async fn test_full_mcp_cycle_tool_list_and_offline_handling() {
    let state = make_offline_state();
    let (sender, _receiver) = channel::create_command_channel(4, state.clone());
    let empty_slot: Arc<std::sync::Mutex<Option<channel::BotCommandReceiver>>> =
        Arc::new(std::sync::Mutex::new(None));
    let server = McpBotServer::new(state.clone(), sender, empty_slot);

    // get_info works even offline
    let info = server.get_info();
    assert_eq!(info.server_info.name, "minecraft-mcp-rs");

    // Query tools return offline error when bot is offline
    assert!(matches!(
        minecraft_mcp_rs::mcp::tools_query::get_self_info(
            &state,
            minecraft_mcp_rs::mcp::tools_query::SelfInfoInput { force: false },
        )
        .await,
        Err(BotError::Offline(_))
    ));
    assert!(matches!(
        minecraft_mcp_rs::mcp::tools_query::get_inventory(
            &state,
            minecraft_mcp_rs::mcp::tools_query::InventoryInput { force: false },
        )
        .await,
        Err(BotError::Offline(_))
    ));
    assert_eq!(
        minecraft_mcp_rs::mcp::tools_query::is_connected(&state).unwrap(),
        r#"{"connected":false}"#
    );
}

// ═══════════════════════════════════════════════════════════════
// Test 2: Channel transmits correct BotCommand for movement
// ═══════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_channel_move_to_sends_correct_position() {
    let (sender, mut receiver) = channel::create_command_channel(4, make_test_state());

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
    let (sender, mut receiver) = channel::create_command_channel(4, make_test_state());

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
    let (sender, mut receiver) = channel::create_command_channel(4, make_test_state());

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
    let (sender, mut receiver) = channel::create_command_channel(4, make_test_state());

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

#[tokio::test]
async fn test_get_self_info_returns_player_data_from_snapshot() {
    let state = make_online_state();
    let result = minecraft_mcp_rs::mcp::tools_query::get_self_info(
        &state,
        minecraft_mcp_rs::mcp::tools_query::SelfInfoInput { force: false },
    )
    .await
    .unwrap();

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

#[tokio::test]
async fn test_get_self_info_offline_returns_error() {
    let state = make_offline_state();
    let result = minecraft_mcp_rs::mcp::tools_query::get_self_info(
        &state,
        minecraft_mcp_rs::mcp::tools_query::SelfInfoInput { force: false },
    )
    .await;
    assert!(matches!(result, Err(BotError::Offline(_))));
}

#[test]
fn test_is_connected_reflects_online_status() {
    let state = make_online_state();
    assert_eq!(
        minecraft_mcp_rs::mcp::tools_query::is_connected(&state).unwrap(),
        r#"{"connected":true}"#
    );

    let offline_state = make_offline_state();
    assert_eq!(
        minecraft_mcp_rs::mcp::tools_query::is_connected(&offline_state).unwrap(),
        r#"{"connected":false}"#
    );
}

// ═══════════════════════════════════════════════════════════════
// Test 4: Concurrent commands are serialized (second waits)
// ═══════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_concurrent_commands_serialized_second_waits() {
    let (sender, mut receiver) = channel::create_command_channel(16, make_test_state());

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
        s2.send_command(BotCommand::WalkDirection(Direction::South, 3))
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
        order[2].contains("WalkDirection"),
        "second processed is WalkDirection"
    );
    assert_eq!(order[3], "end-second");
}

#[tokio::test]
async fn test_multiple_commands_all_get_responses() {
    let (sender, mut receiver) = channel::create_command_channel(16, make_test_state());

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
    let (sender, receiver) = channel::create_command_channel(4, make_test_state());
    drop(receiver);

    let result = sender.send_command(BotCommand::Jump).await;

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
    let (sender, receiver) = channel::create_command_channel(4, make_test_state());
    drop(receiver);

    let commands = vec![
        BotCommand::Jump,
        BotCommand::MoveTo(BlockPos::new(0, 0, 0)),
        BotCommand::BreakBlock(BlockPos::new(0, 0, 0)),
        BotCommand::SendChat("hello".into()),
        BotCommand::QueryInventory,
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

    assert!(matches!(
        minecraft_mcp_rs::mcp::tools_query::get_self_info(
            &state,
            minecraft_mcp_rs::mcp::tools_query::SelfInfoInput { force: false },
        )
        .await,
        Err(BotError::Offline(_))
    ));
    assert!(matches!(
        minecraft_mcp_rs::mcp::tools_query::get_inventory(
            &state,
            minecraft_mcp_rs::mcp::tools_query::InventoryInput { force: false },
        )
        .await,
        Err(BotError::Offline(_))
    ));
    assert!(matches!(
        minecraft_mcp_rs::mcp::tools_query::get_nearby_blocks(&state, 10, None),
        Err(BotError::Offline(_))
    ));
    assert!(matches!(
        minecraft_mcp_rs::mcp::tools_query::get_nearby_entities(&state, 10),
        Err(BotError::Offline(_))
    ));
    assert!(matches!(
        minecraft_mcp_rs::mcp::tools_query::get_chunk_summary(&state),
        Err(BotError::Offline(_))
    ));
    // get_world_view (multi-content: image + annotation) is also gated on
    // online status — offline must yield Offline, not a render attempt.
    assert!(matches!(
        minecraft_mcp_rs::mcp::tools_query::get_world_view(&state, 8, 2),
        Err(BotError::Offline(_))
    ));
}

// ═══════════════════════════════════════════════════════════════
// Test 6: Responder-dropped returns Offline (channel closed), distinct from
//         receiver-dropped (also Offline but different message) and from
//         genuine CommandTimeout (responder alive but slow).
// ═══════════════════════════════════════════════════════════════

/// When the receiver task accepts a command but drops the oneshot responder
/// without replying (e.g. executor aborted mid-command), the sender observes
/// a closed channel and reports `BotError::Offline` — not `CommandTimeout`,
/// because the responder is gone permanently rather than merely slow.
#[tokio::test]
async fn test_offline_returned_when_responder_dropped() {
    let (sender, mut receiver) = channel::create_command_channel(4, make_test_state());

    let dropper = tokio::spawn(async move {
        let wrapped = receiver.recv().await.expect("should receive command");
        drop(wrapped);
    });

    let result = sender.send_command(BotCommand::Jump).await;

    match result {
        Err(BotError::Offline(msg)) => {
            assert!(
                msg.contains("responder dropped"),
                "expected message to mention responder dropped, got: {msg}"
            );
        }
        other => panic!("expected BotError::Offline, got: {:?}", other),
    }

    dropper.await.expect("dropper should complete");
}

/// Same as above but for `BreakBlock` — verifies the command payload is
/// delivered to the receiver before the responder is dropped.
#[tokio::test]
async fn test_offline_returned_when_responder_dropped_break_block() {
    let (sender, mut receiver) = channel::create_command_channel(4, make_test_state());

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
        Err(BotError::Offline(msg)) => {
            assert!(
                msg.contains("responder dropped"),
                "expected message to mention responder dropped, got: {msg}"
            );
        }
        other => panic!("expected Offline, got: {:?}", other),
    }

    dropper.await.expect("dropper should complete");
}

/// Both `Offline` causes should yield `BotError::Offline`, but with
/// distinguishable messages so users can tell whether the receiver was
/// never there ("channel closed") or accepted the command then died
/// ("responder dropped").
#[tokio::test]
async fn test_offline_messages_distinguish_receiver_dropped_vs_responder_dropped() {
    // Case A: receiver dropped before command is sent.
    let (sender1, receiver1) = channel::create_command_channel(4, make_test_state());
    drop(receiver1);
    let offline_a = sender1.send_command(BotCommand::Jump).await;
    let msg_a = match offline_a {
        Err(BotError::Offline(m)) => m,
        other => panic!("expected Offline for receiver-dropped, got: {:?}", other),
    };
    assert!(msg_a.contains("closed"), "expected 'closed' in: {msg_a}");

    // Case B: receiver exists but drops responder without replying.
    let (sender2, mut receiver2) = channel::create_command_channel(4, make_test_state());
    let dropper = tokio::spawn(async move {
        let wrapped = receiver2.recv().await.unwrap();
        drop(wrapped);
    });
    let offline_b = sender2.send_command(BotCommand::Jump).await;
    let msg_b = match offline_b {
        Err(BotError::Offline(m)) => m,
        other => panic!("expected Offline for responder-dropped, got: {:?}", other),
    };
    assert!(
        msg_b.contains("responder dropped"),
        "expected 'responder dropped' in: {msg_b}"
    );
    assert_ne!(msg_a, msg_b, "the two Offline messages should differ");

    dropper.await.unwrap();
}

/// When the receiver task accepts a command but takes longer than
/// `command_timeout_secs` to reply, the sender must observe
/// `BotError::CommandTimeout` — distinct from `BotError::Offline`,
/// which is reserved for the responder-dropped / receiver-dropped
/// cases. The responder is kept **alive** (we sleep rather than drop)
/// so the genuine `tokio::time::timeout` branch is exercised, not the
/// responder-dropped branch.
#[tokio::test]
async fn test_command_timeout_responder_alive_but_slow() {
    let state = make_offline_state();
    // Set a 1-second timeout (smallest representable value with
    // the current `AppConfig::command_timeout_secs: u64` field).
    state.update_config(|cfg| {
        cfg.command_timeout_secs = 1;
    });
    let (sender, mut receiver) = channel::create_command_channel(4, state);

    let cmd = BotCommand::Jump;

    let responder = tokio::spawn(async move {
        let wrapped = receiver.recv().await.expect("should receive command");
        // Hold the responder for 1.5s — longer than the 1s timeout —
        // so send_command's `tokio::time::timeout` fires first.
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        // Drop after the timeout has already fired (irrelevant to the
        // assertion, just keeps the task clean).
        drop(wrapped);
    });

    let result = sender.send_command(cmd).await;

    match result {
        Err(BotError::CommandTimeout { command, .. }) => {
            assert!(
                command.contains("Jump"),
                "expected command field to mention Jump, got: {command}"
            );
        }
        other => panic!("expected BotError::CommandTimeout, got: {:?}", other),
    }

    responder.await.expect("responder should complete");
}

// ═══════════════════════════════════════════════════════════════
// Test 7: Auto-reconnect sequence simulation
// ═══════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_auto_reconnect_sequence_simulation() {
    let state = make_online_state();
    let (sender, mut receiver) = channel::create_command_channel(4, state.clone());

    let _chan_task = tokio::spawn(async move {
        while let Some(wrapped) = receiver.recv().await {
            let _ = wrapped.respond_to.send(Ok(bot_result(true, "ok")));
        }
    });

    let empty_slot: Arc<std::sync::Mutex<Option<channel::BotCommandReceiver>>> =
        Arc::new(std::sync::Mutex::new(None));
    let _server = McpBotServer::new(state.clone(), sender.clone(), empty_slot);

    // Phase 1: Online
    assert_eq!(
        minecraft_mcp_rs::mcp::tools_query::is_connected(&state).unwrap(),
        r#"{"connected":true}"#
    );
    let response = minecraft_mcp_rs::mcp::tools_query::get_self_info(
        &state,
        minecraft_mcp_rs::mcp::tools_query::SelfInfoInput { force: false },
    )
    .await
    .unwrap();
    assert!(response.contains("TestBot"));

    // Phase 2: Disconnect
    state.set_online(false);
    assert_eq!(
        minecraft_mcp_rs::mcp::tools_query::is_connected(&state).unwrap(),
        r#"{"connected":false}"#
    );
    let offline_resp = minecraft_mcp_rs::mcp::tools_query::get_self_info(
        &state,
        minecraft_mcp_rs::mcp::tools_query::SelfInfoInput { force: false },
    )
    .await;
    assert!(matches!(offline_resp, Err(BotError::Offline(_))));

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
            position_precise: None,
            yaw: None,
        },
        timestamp: 99,
        chunk_summary: vec![(1, 1)],
        commands_enabled: None,
        ..Default::default()
    };
    state.update_snapshot(fresh_snap);

    assert_eq!(
        minecraft_mcp_rs::mcp::tools_query::is_connected(&state).unwrap(),
        r#"{"connected":true}"#
    );

    let reconnected = minecraft_mcp_rs::mcp::tools_query::get_self_info(
        &state,
        minecraft_mcp_rs::mcp::tools_query::SelfInfoInput { force: false },
    )
    .await
    .unwrap();
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
    let (sender, mut receiver) = channel::create_command_channel(4, state.clone());

    let _chan_task = tokio::spawn(async move {
        while let Some(wrapped) = receiver.recv().await {
            let _ = wrapped.respond_to.send(Ok(bot_result(true, "ok")));
        }
    });

    let empty_slot: Arc<std::sync::Mutex<Option<channel::BotCommandReceiver>>> =
        Arc::new(std::sync::Mutex::new(None));
    let _server = McpBotServer::new(state.clone(), sender.clone(), empty_slot);

    for cycle in 0..3 {
        state.set_online(true);
        assert_eq!(
            minecraft_mcp_rs::mcp::tools_query::is_connected(&state).unwrap(),
            r#"{"connected":true}"#,
            "cycle {cycle}: should be online"
        );

        state.set_online(false);
        assert_eq!(
            minecraft_mcp_rs::mcp::tools_query::is_connected(&state).unwrap(),
            r#"{"connected":false}"#,
            "cycle {cycle}: should be offline"
        );
        let offline_resp = minecraft_mcp_rs::mcp::tools_query::get_self_info(
            &state,
            minecraft_mcp_rs::mcp::tools_query::SelfInfoInput { force: false },
        )
        .await;
        assert!(
            matches!(offline_resp, Err(BotError::Offline(_))),
            "cycle {cycle}: offline should return error"
        );
    }
}

// ═══════════════════════════════════════════════════════════════
// Test 8: All MCP tool functions exist and no craft_item
// ═══════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_all_query_tools_exist_and_work() {
    let state = make_online_state();

    let self_info = minecraft_mcp_rs::mcp::tools_query::get_self_info(
        &state,
        minecraft_mcp_rs::mcp::tools_query::SelfInfoInput { force: false },
    )
    .await
    .unwrap();
    assert!(!self_info.is_empty());

    let inventory = minecraft_mcp_rs::mcp::tools_query::get_inventory(
        &state,
        minecraft_mcp_rs::mcp::tools_query::InventoryInput { force: false },
    )
    .await
    .unwrap();
    assert!(inventory.contains("held_item_slot"));

    let nearby_blocks =
        minecraft_mcp_rs::mcp::tools_query::get_nearby_blocks(&state, 1, None).unwrap();
    assert!(!nearby_blocks.is_empty());

    let nearby_entities =
        minecraft_mcp_rs::mcp::tools_query::get_nearby_entities(&state, 1).unwrap();
    assert!(!nearby_entities.is_empty());

    let chunk_summary = minecraft_mcp_rs::mcp::tools_query::get_chunk_summary(&state).unwrap();
    assert!(!chunk_summary.is_empty());

    let connected = minecraft_mcp_rs::mcp::tools_query::is_connected(&state).unwrap();
    assert_eq!(connected, r#"{"connected":true}"#);
}

#[tokio::test]
async fn test_all_bot_command_variants_exist_no_craft_item() {
    let (sender, mut receiver) = channel::create_command_channel(16, make_test_state());

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
        BotCommand::UseItemWithSlot(0),
        BotCommand::EquipTool(ToolType::Pickaxe),
        BotCommand::EquipToolWithMaterial(ToolType::Pickaxe, MaterialTier::Diamond),
        BotCommand::OpenContainer(BlockPos::new(0, 64, 0)),
        BotCommand::TakeFromContainer(0, 1),
        BotCommand::PutIntoContainer(0, 1),
        BotCommand::CloseContainer,
        BotCommand::AttackEntity(42),
        BotCommand::ShieldBlock(true),
        BotCommand::SendChat("test".into()),
        BotCommand::ExecuteCommand("/help".into()),
        BotCommand::SetGameMode(GameMode::Survival),
        BotCommand::QueryInventory,
        // ── v2 foundation: extended capabilities ──────────────────
        BotCommand::SmartMove(BlockPos::new(0, 64, 0)),
        BotCommand::FlyTo(BlockPos::new(0, 64, 0)),
        BotCommand::CollectItems(5),
        BotCommand::Act(ActAction::Move {
            target: BlockPos::new(0, 64, 0),
        }),
    ];

    assert_eq!(
        commands.len(),
        27,
        "should have exactly 27 BotCommand variants (7 dead Query* variants removed in 1.1.0)"
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

    // Verify all 6 ActAction sub-variants can be serialized into BotCommand::Act
    let act_variants = vec![
        ActAction::Move {
            target: BlockPos::new(1, 64, 1),
        },
        ActAction::SmartMove {
            target: BlockPos::new(2, 64, 2),
        },
        ActAction::Fly {
            target: BlockPos::new(3, 64, 3),
        },
        ActAction::Mine {
            block_pos: BlockPos::new(4, 64, 4),
        },
        ActAction::Attack { entity_id: 7 },
        ActAction::CollectItems { radius: 8 },
    ];
    assert_eq!(
        act_variants.len(),
        6,
        "ActAction should have exactly 6 sub-variants"
    );
    for action in act_variants {
        let cmd = BotCommand::Act(action);
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(
            !json.to_lowercase().contains("craft_item"),
            "Act serialization must not contain craft_item"
        );
    }

    drop(sender);
    let variants = responder.await.expect("responder should finish");
    assert!(
        !variants.contains("CraftItem"),
        "no CraftItem variant should exist"
    );
}

// ═══════════════════════════════════════════════════════════════
// Test 9: Action tool handlers propagate BotError variants from the channel
// ═══════════════════════════════════════════════════════════════

/// When the channel receiver is dropped, `send_command` returns
/// `BotError::Offline`. The MCP tool handler must propagate that variant
/// directly so the client receives JSON-RPC code `-32000` with
/// `reason: "bot_disconnected"`. Wrapping it in `BotError::Internal`
/// would hide the disconnection from the MCP client.
#[tokio::test]
async fn test_send_chat_propagates_channel_offline_error() {
    let state = make_online_state();
    let (sender, receiver) = channel::create_command_channel(4, state.clone());
    drop(receiver);

    let result =
        minecraft_mcp_rs::mcp::tools_chat::handle_send_chat(&state, &sender, "hello".into()).await;
    assert!(
        matches!(result, Err(BotError::Offline(_))),
        "expected Offline error to propagate, got {result:?}"
    );
}

#[tokio::test]
async fn test_break_block_propagates_channel_offline_error() {
    let state = make_online_state();
    let (sender, receiver) = channel::create_command_channel(4, state.clone());
    drop(receiver);

    let input = minecraft_mcp_rs::mcp::tools_block::BreakBlockInput {
        x: 0,
        y: 64,
        z: 0,
        use_best_tool: None,
    };
    let result =
        minecraft_mcp_rs::mcp::tools_block::handle_break_block(&state, &sender, input).await;
    assert!(
        matches!(result, Err(BotError::Offline(_))),
        "expected Offline error to propagate, got {result:?}"
    );
}

#[tokio::test]
async fn test_move_to_propagates_channel_offline_error() {
    let state = make_online_state();
    let (sender, receiver) = channel::create_command_channel(4, state.clone());
    drop(receiver);

    let input = minecraft_mcp_rs::mcp::tools_movement::MoveToInput { x: 1, y: 64, z: 1 };
    let result =
        minecraft_mcp_rs::mcp::tools_movement::handle_move_to(&state, &sender, input).await;
    assert!(
        matches!(result, Err(BotError::Offline(_))),
        "expected Offline error to propagate, got {result:?}"
    );
}

/// When the responder takes longer than `command_timeout_secs` to reply,
/// `send_command` returns `BotError::CommandTimeout`. The handler must
/// propagate it directly instead of converting it to `BotError::Internal`.
#[tokio::test]
async fn test_send_chat_propagates_command_timeout() {
    let state = make_online_state();
    state.update_config(|cfg| {
        cfg.command_timeout_secs = 1;
    });
    let (sender, mut receiver) = channel::create_command_channel(4, state.clone());

    let responder = tokio::spawn(async move {
        let wrapped = receiver.recv().await.expect("should receive command");
        // Hold the responder for 1.5s, longer than the 1s timeout.
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        drop(wrapped);
    });

    let result =
        minecraft_mcp_rs::mcp::tools_chat::handle_send_chat(&state, &sender, "hello".into()).await;
    assert!(
        matches!(result, Err(BotError::CommandTimeout { .. })),
        "expected CommandTimeout error to propagate, got {result:?}"
    );

    responder.await.expect("responder should complete");
}

// ═══════════════════════════════════════════════════════════════
// Additional: channel factory, sender cloning, compound ops
// ═══════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_channel_factory_creates_working_pair() {
    let (sender, mut receiver) = channel::create_command_channel(8, make_test_state());

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
    let (sender, mut receiver) = channel::create_command_channel(8, make_test_state());
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
    let (sender, mut receiver) = channel::create_command_channel(16, make_test_state());

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

// ═══════════════════════════════════════════════════════════════
// Test 10: collect_items sees item entities rebuilt from the live ECS
// (F6-2)
// ═══════════════════════════════════════════════════════════════

/// F6-2 regression: entities are rebuilt from the live ECS on every
/// snapshot tick by `SnapshotUpdater`, so dropped items actually appear
/// in `WorldSnapshot::entities`. With an `item` entity within radius the
/// MCP `collect_items` handler must relay `BotCommand::CollectItems` and
/// propagate a `visited >= 1` result instead of the old
/// "No items to collect".
#[tokio::test]
async fn test_collect_items_mcp_flow_with_item_entities() {
    let state = SharedState::new(AppConfig::default());
    state.set_online(true);

    // Snapshot shaped like what SnapshotUpdater::collect_entities now
    // produces: the player at the origin and a dropped item 2 blocks away.
    let snap = WorldSnapshot {
        entities: vec![EntityEntry {
            id: 11,
            uuid: "item-uuid".into(),
            entity_type: "item".into(),
            position: BlockPos::new(2, 64, 1),
            display_name: None,
            health: None,
        }],
        self_player: SelfPlayer {
            uuid: "player-uuid".into(),
            username: "TestBot".into(),
            position: BlockPos::new(0, 64, 0),
            health: 20.0,
            hunger: 20,
            gamemode: GameMode::Survival,
            held_item_slot: 0,
            inventory: Vec::new(),
            position_precise: None,
            yaw: None,
        },
        ..Default::default()
    };
    state.update_snapshot(snap);
    let state = Arc::new(state);

    let (sender, mut receiver) = channel::create_command_channel(4, state.clone());

    // Mock executor: assert the command arrives with the requested radius
    // and reply with the result the real executor produces when it found
    // and visited one item entity.
    let responder = tokio::spawn(async move {
        let wrapped = receiver.recv().await.expect("should receive CollectItems");
        match wrapped.command {
            BotCommand::CollectItems(radius) => assert_eq!(radius, 5),
            other => panic!("expected CollectItems, got: {:?}", other),
        }
        let _ = wrapped.respond_to.send(Ok(BotResult {
            success: true,
            message: "Visited 1 item drop location(s); auto-pickup expected on proximity".into(),
            data: Some(serde_json::json!({"visited": 1})),
        }));
    });

    let input = minecraft_mcp_rs::mcp::tools_item::CollectItemsInput { radius: 5 };
    let result = minecraft_mcp_rs::mcp::tools_item::handle_collect_items(&state, &sender, input)
        .await
        .expect("collect_items should succeed");

    assert!(
        result.contains("Visited 1"),
        "expected a visited-1 result, got: {result}"
    );
    assert!(
        result.contains("\"visited\":1"),
        "expected visited payload, got: {result}"
    );

    drop(sender);
    responder.await.expect("responder should complete");
}

/// Same F6-2 fix at the query layer: `get_nearby_entities` must return
/// non-player entries (item drops, mobs) once the snapshot entities are
/// rebuilt from the live ECS.
#[tokio::test]
async fn test_get_nearby_entities_includes_item_drops() {
    let state = SharedState::new(AppConfig::default());
    state.set_online(true);

    let snap = WorldSnapshot {
        entities: vec![
            EntityEntry {
                id: 21,
                uuid: "drop-uuid".into(),
                entity_type: "item".into(),
                position: BlockPos::new(1, 64, 0),
                display_name: None,
                health: None,
            },
            EntityEntry {
                id: 22,
                uuid: "player-uuid-2".into(),
                entity_type: "player".into(),
                position: BlockPos::new(2, 64, 0),
                display_name: Some("Steve".into()),
                health: Some(20.0),
            },
        ],
        self_player: SelfPlayer {
            uuid: "player-uuid".into(),
            username: "TestBot".into(),
            position: BlockPos::new(0, 64, 0),
            health: 20.0,
            hunger: 20,
            gamemode: GameMode::Survival,
            held_item_slot: 0,
            inventory: Vec::new(),
            position_precise: None,
            yaw: None,
        },
        ..Default::default()
    };
    state.update_snapshot(snap);
    let state = Arc::new(state);

    let result = minecraft_mcp_rs::mcp::tools_query::get_nearby_entities(&state, 8)
        .expect("get_nearby_entities should succeed");
    assert!(
        result.contains("\"entity_type\":\"item\""),
        "item drop must be listed: {result}"
    );
    assert!(
        result.contains("\"entity_type\":\"player\""),
        "other players must still be listed: {result}"
    );
}

// ═══════════════════════════════════════════════════════════════
// Settings & lifecycle tools (T13)
// ═══════════════════════════════════════════════════════════════

/// `get_settings` works while the bot is offline and always redacts the MCP
/// token to `"***"` — the real token must never appear in the output.
#[test]
fn test_get_settings_works_offline_and_redacts_token() {
    let state = make_offline_state();
    let result = minecraft_mcp_rs::mcp::tools_settings::get_settings(&state)
        .expect("get_settings must work offline");
    assert!(
        result.contains("\"mcp_token\": \"***\""),
        "token must be redacted: {result}"
    );
    let real_token = state.read_config().mcp_token.clone();
    assert!(
        !result.contains(&real_token),
        "real token must never leak: {result}"
    );
    assert!(result.contains("\"online\": false"), "got: {result}");
    assert!(
        result.contains("\"config_path\":"),
        "runtime block should include config_path: {result}"
    );
}

/// `update_settings` with an invalid port fails with `InvalidParams` before
/// anything is persisted (validation happens before the disk write).
#[test]
fn test_update_settings_invalid_port_rejected() {
    let state = make_offline_state();
    let input = minecraft_mcp_rs::mcp::tools_settings::UpdateSettingsInput {
        mc_port: Some(0),
        ..Default::default()
    };
    let result = minecraft_mcp_rs::mcp::tools_settings::update_settings(&state, input);
    match result {
        Err(BotError::InvalidParams(msg)) => {
            assert!(msg.contains("mc_port"), "unexpected message: {msg}")
        }
        other => panic!("expected InvalidParams, got: {other:?}"),
    }
}

/// `update_settings` applies a valid partial update in memory and persists it
/// to the real config path (the tool always uses it). The pre-existing config
/// file — if any — is restored afterwards so the test never leaves the host's
/// real settings clobbered.
#[test]
fn test_update_settings_applies_valid_input() {
    // Snapshot any pre-existing config so we can restore it after the test.
    let cfg_path = minecraft_mcp_rs::config::config_path()
        .expect("OS config dir should be discoverable on the test host");
    let had_original = cfg_path.exists();
    let original = if had_original {
        Some(std::fs::read_to_string(&cfg_path).expect("read pre-existing config"))
    } else {
        None
    };

    let state = make_offline_state();
    let input = minecraft_mcp_rs::mcp::tools_settings::UpdateSettingsInput {
        task_name: Some("itest-task".into()),
        ..Default::default()
    };
    let result = minecraft_mcp_rs::mcp::tools_settings::update_settings(&state, input)
        .expect("valid update should succeed");
    let v: serde_json::Value =
        serde_json::from_str(&result).expect("update response must be valid JSON");
    assert_eq!(
        v["applied"]["task_name"], "itest-task",
        "response must report the applied field: {result}"
    );
    assert_eq!(v["persisted"], true, "response must report persistence");
    assert_eq!(
        state.read_config().task_name,
        "itest-task",
        "in-memory config must reflect the applied update"
    );

    // Restore the host's real config file (or remove the one we created).
    match original {
        Some(content) => std::fs::write(&cfg_path, content).expect("restore config"),
        None => {
            let _ = std::fs::remove_file(&cfg_path);
        }
    }
}

/// `connect_bot` spawns a connection thread even while the bot is offline,
/// setting the connecting flag; requesting a disconnect makes the loop stop
/// and the flag clears when the thread exits.
#[tokio::test]
async fn test_connect_bot_offline_spawns_connection() {
    let state = make_offline_state();
    // Point at a port nothing listens on so the connection attempt fails
    // fast (no real network dependency in tests).
    state.update_config(|cfg| cfg.mc_port = 1);
    let (sender, _receiver) = channel::create_command_channel(4, state.clone());
    let slot: Arc<std::sync::Mutex<Option<channel::BotCommandReceiver>>> =
        Arc::new(std::sync::Mutex::new(None));

    let result = minecraft_mcp_rs::mcp::tools_settings::connect_bot(&state, &slot, &sender)
        .expect("connect_bot should return Ok");
    assert!(result.contains("connection started"), "got: {result}");
    assert!(state.is_connecting(), "connecting flag must be set");

    // Tear down: keep requesting a disconnect while polling the thread.
    // A single `request_disconnect()` before the thread starts would be
    // consumed by `connect()`'s startup `clear_disconnect_request()`; the
    // repeated requests land during the loop's backoff sleep and the
    // cancellation token makes the loop exit promptly.
    let handle = state
        .take_bot_thread_handle()
        .expect("handle must be stored");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while !handle.is_finished() && std::time::Instant::now() < deadline {
        state.request_disconnect();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    if !handle.is_finished() {
        panic!("bot thread did not exit after disconnect request");
    }
    assert!(
        !state.is_connecting(),
        "connecting flag must be cleared after thread exit"
    );
}

/// `disconnect_bot` requests a disconnect (idempotent offline no-op).
#[test]
fn test_disconnect_bot_requests_disconnect() {
    let state = make_offline_state();
    let result = minecraft_mcp_rs::mcp::tools_settings::disconnect_bot(&state)
        .expect("disconnect_bot should succeed");
    assert!(result.contains("disconnect requested"), "got: {result}");
    assert!(state.is_disconnect_requested());
}
