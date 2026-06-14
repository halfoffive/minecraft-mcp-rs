//! Minecraft MCP Server — binary entry point.
//!
//! Architecture:
//! - Main thread: spawns tokio runtime in background for MCP, runs egui UI.
//! - Background thread: own tokio runtime, runs MCP server on stdio transport.
//! - All logs → stderr, stdout = MCP channel only.
//!
//! Shared state is accessed lock-free by both threads.

// Module declarations for tests in the binary crate.
mod block_data;
mod config;
pub mod logging;
pub mod types;

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
    // Create shared state and command channel outside any runtime.
    // Tokio mpsc channels can be created without an active runtime;
    // only `send` operations (which are async) need the runtime.
    // ══════════════════════════════════════════════════════════════════
    let config = AppConfig::default();
    let state = Arc::new(SharedState::new(config));
    let (sender, _receiver) = channel::create_command_channel(64);

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

            // EnterGuard sets this runtime as the current one for the
            // duration of the guard, enabling `tokio::spawn` calls inside
            // `serve_stdio`.
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

    // ══════════════════════════════════════════════════════════════════
    // Run the egui UI on the main thread.  This call blocks until the
    // window is closed, at which point the process exits.
    // ══════════════════════════════════════════════════════════════════
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([640.0, 480.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Minecraft MCP Server",
        native_options,
        Box::new(move |_cc| {
            Ok(Box::new(MinecraftApp::new(
                state_for_egui,
                sender_for_egui,
            )))
        }),
    )
    .expect("Failed to start egui");
}
