//! Minecraft MCP Server — binary entry point.
//!
//! Architecture:
//! - Main thread: runs egui UI.
//! - MCP server thread: own tokio runtime, runs MCP on stdio transport.
//! - Bot connection thread: spawned on demand from the UI, own tokio runtime.
//! - All logs → stderr, stdout = MCP channel only.
//!
//! Shared state is accessed lock-free by all threads.

use std::sync::Arc;

use minecraft_mcp_rs::channel;
use minecraft_mcp_rs::config::AppConfig;
use minecraft_mcp_rs::logging::init_logging;
use minecraft_mcp_rs::mcp::server::serve_stdio;
use minecraft_mcp_rs::state::SharedState;
use minecraft_mcp_rs::ui::app::MinecraftApp;

/// `main` is **not** `async` — the egui event loop runs on the main thread,
/// and the MCP server runs on a background thread with its own tokio runtime.
fn main() {
    // ══════════════════════════════════════════════════════════════════
    // Logging must be initialized FIRST — all subsequent output goes to
    // stderr only. Stdout is reserved for the MCP JSON-RPC transport.
    // ══════════════════════════════════════════════════════════════════
    init_logging();

    tracing::info!("Minecraft MCP server starting");

    // ══════════════════════════════════════════════════════════════════
    // Create shared state and command channel.
    // Tokio mpsc channels can be created without an active runtime;
    // only `send` operations (which are async) need the runtime.
    // ══════════════════════════════════════════════════════════════════
    let config = AppConfig::default();
    let state = Arc::new(SharedState::new(config.clone()));
    let (sender, receiver) = channel::create_command_channel(64);
    // Honour the user-configurable command timeout (editable in the UI).
    let sender = sender.with_timeout(std::time::Duration::from_secs(config.command_timeout_secs));
    // Wrap the receiver in a shared slot (Arc<Mutex<Option<_>>>) so the
    // azalea event handler can lease it on `Event::Spawn` and return it to
    // the slot when the executor is aborted on disconnect. This keeps the
    // receiver alive across reconnection attempts.
    let receiver: Arc<std::sync::Mutex<Option<channel::BotCommandReceiver>>> =
        Arc::new(std::sync::Mutex::new(Some(receiver)));

    // ══════════════════════════════════════════════════════════════════
    // Clone for the background MCP thread.
    // ══════════════════════════════════════════════════════════════════
    let state_for_mcp = Arc::clone(&state);
    let sender_for_mcp = sender.clone();

    // ══════════════════════════════════════════════════════════════════
    // Spawn the MCP server on a dedicated OS thread with its own tokio
    // runtime.  The EnterGuard ensures that `tokio::spawn` and other
    // runtime-dependent operations work within the `block_on` scope.
    // ══════════════════════════════════════════════════════════════════
    std::thread::Builder::new()
        .name("mcp-server".into())
        .spawn(move || {
            let rt = tokio::runtime::Runtime::new()
                .expect("Failed to create tokio runtime for MCP server");

            let _guard = rt.enter();

            rt.block_on(async {
                serve_stdio(state_for_mcp, sender_for_mcp).await;
            });

            tracing::info!("MCP server thread exited");
        })
        .expect("Failed to spawn MCP server thread");

    // ══════════════════════════════════════════════════════════════════
    // Clone for the egui closure (moved into FnOnce).
    // ══════════════════════════════════════════════════════════════════
    let state_for_egui = Arc::clone(&state);
    let sender_for_egui = sender.clone();
    let receiver_for_egui = Arc::clone(&receiver);

    // ══════════════════════════════════════════════════════════════════
    // Run the egui UI on the main thread.  This call blocks until the
    // window is closed, at which point the process exits.
    // ══════════════════════════════════════════════════════════════════
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([780.0, 560.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Minecraft MCP Server",
        native_options,
        Box::new(move |_cc| {
            Ok(Box::new(MinecraftApp::new(
                state_for_egui,
                sender_for_egui,
                receiver_for_egui,
            )))
        }),
    )
    .expect("Failed to start egui");
}
