//! Minecraft MCP Server — binary entry point.
//!
//! Two modes:
//! - **UI mode** (no CLI flags): runs the egui desktop window on the main
//!   thread. The window's Connect button spawns the bot connection; closing
//!   the window shuts everything down.
//! - **Headless mode** (`--headless`): no window. A supervisor thread
//!   auto-connects the bot on startup and re-spawns the connection after
//!   agent-driven config changes. The process exits when the MCP transport
//!   closes (stdio client gone) or on shutdown.
//!
//! Architecture:
//! - Main thread: egui UI (UI mode) or supervisor orchestration (headless).
//! - MCP server thread: own tokio runtime, runs MCP on stdio or HTTP
//!   transport.
//! - Bot connection thread: spawned on demand (UI Connect button, the
//!   `connect_bot` MCP tool, or the headless supervisor), own tokio runtime.
//! - All logs → stderr, stdout = MCP channel only.
//!
//! Shared state is accessed lock-free by all threads.

// Hide the console window in release builds on Windows. Debug builds retain
// the console for diagnostic output (tracing logs, panics, etc.).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use minecraft_mcp_rs::channel;
use minecraft_mcp_rs::config::{AppConfig, McpTransport};
use minecraft_mcp_rs::logging::init_logging;
use minecraft_mcp_rs::mcp::server::{serve_http, serve_stdio};
use minecraft_mcp_rs::state::SharedState;
use minecraft_mcp_rs::ui::app::MinecraftApp;

/// `main` is **not** `async` — the egui event loop (UI mode) or the headless
/// supervisor runs on the main thread, and the MCP server runs on a
/// background thread with its own tokio runtime.
fn main() {
    // ══════════════════════════════════════════════════════════════════
    // Logging must be initialized FIRST — all subsequent output goes to
    // stderr only. Stdout is reserved for the MCP JSON-RPC transport.
    // ══════════════════════════════════════════════════════════════════
    init_logging();

    tracing::info!("Minecraft MCP server starting");

    // ══════════════════════════════════════════════════════════════════
    // Parse CLI arguments. Help prints to stderr and exits 0; a parse
    // error prints to stderr and exits 2 (stdout stays clean for the MCP
    // transport either way).
    // ══════════════════════════════════════════════════════════════════
    let args = match minecraft_mcp_rs::cli::parse_cli_args(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    if args.help {
        minecraft_mcp_rs::cli::print_help();
        return;
    }

    // ══════════════════════════════════════════════════════════════════
    // Load the config file (explicit path via --config, else the OS config
    // dir), then apply CLI overrides. `--stdio` forces the stdio MCP
    // transport so `npx minecraft-mcp-rs --headless --stdio` works
    // regardless of what the config file says.
    // ══════════════════════════════════════════════════════════════════
    let mut config = AppConfig::load_from_disk(args.config_path.as_deref());
    if args.force_stdio {
        config.mcp_transport = McpTransport::Stdio;
    }
    // Set the active i18n language from the persisted/default config BEFORE
    // constructing any UI strings (notably the window title passed to
    // `eframe::run_native` below).  This ensures the title and all UI text
    // honour the user's saved language from the very first frame.
    minecraft_mcp_rs::ui::i18n::set(config.language);

    // ══════════════════════════════════════════════════════════════════
    // Create shared state and command channel.
    // Tokio mpsc channels can be created without an active runtime;
    // only `send` operations (which are async) need the runtime.
    // ══════════════════════════════════════════════════════════════════
    let state = Arc::new(SharedState::new(config.clone()));
    // Wire the shared state into the channel so `BotCommandSender` can
    // hot-read `command_timeout_secs` from the UI on every send — the
    // sender no longer holds a stale `Duration` of its own.
    let (sender, receiver) = channel::create_command_channel(64, Arc::clone(&state));
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
    let receiver_for_mcp = Arc::clone(&receiver);
    let headless = args.headless;

    // ══════════════════════════════════════════════════════════════════
    // Spawn the MCP server on a dedicated OS thread with its own tokio
    // runtime.  The EnterGuard ensures that `tokio::spawn` and other
    // runtime-dependent operations work within the `block_on` scope.
    //
    // The JoinHandle is captured so the UI (`MinecraftApp::drop`) or the
    // headless main thread can join it after shutdown.
    // ══════════════════════════════════════════════════════════════════
    let mcp_handle: std::thread::JoinHandle<()> = std::thread::Builder::new()
        .name("mcp-server".into())
        .spawn(move || {
            let rt = tokio::runtime::Runtime::new()
                .expect("Failed to create tokio runtime for MCP server");

            let _guard = rt.enter();

            rt.block_on(async {
                let transport = state_for_mcp.read_config().mcp_transport;
                match transport {
                    McpTransport::Stdio => {
                        serve_stdio(state_for_mcp.clone(), sender_for_mcp, receiver_for_mcp).await;
                    }
                    McpTransport::Http => {
                        let (port, mcp_address) = {
                            let config = state_for_mcp.read_config();
                            (config.mcp_port, config.mcp_address.clone())
                        };
                        let ip: IpAddr = mcp_address.parse().unwrap_or_else(|_| {
                            tracing::warn!(
                                address = %mcp_address,
                                "failed to parse mcp_address as IP, falling back to 127.0.0.1"
                            );
                            IpAddr::V4(Ipv4Addr::LOCALHOST)
                        });
                        let addr = SocketAddr::new(ip, port);
                        serve_http(
                            state_for_mcp.clone(),
                            sender_for_mcp,
                            receiver_for_mcp,
                            addr,
                        )
                        .await;
                    }
                }

                // Headless stdio convention: the MCP transport closing means
                // the client is gone — trigger the shutdown token so the
                // supervisor exits and the process terminates. In UI mode
                // the window's Drop impl handles shutdown instead.
                if headless {
                    tracing::info!("headless mode: MCP transport closed — triggering shutdown");
                    state_for_mcp.trigger_shutdown();
                }
            });

            tracing::info!("MCP server thread exited");
        })
        .expect("Failed to spawn MCP server thread");

    // ══════════════════════════════════════════════════════════════════
    // Headless mode: run the supervisor instead of the egui window.
    // ══════════════════════════════════════════════════════════════════
    if headless {
        let state_for_supervisor = Arc::clone(&state);
        let sender_for_supervisor = sender.clone();
        let receiver_for_supervisor = Arc::clone(&receiver);

        let supervisor_handle: std::thread::JoinHandle<()> = std::thread::Builder::new()
            .name("headless-supervisor".into())
            .spawn(move || {
                minecraft_mcp_rs::bot::spawn::headless_supervisor(
                    state_for_supervisor,
                    receiver_for_supervisor,
                    sender_for_supervisor,
                );
            })
            .expect("Failed to spawn headless supervisor thread");

        // Block until the supervisor exits (shutdown token cancelled by the
        // MCP thread once the transport closes).
        let _ = supervisor_handle.join();
        // Bound the final join of the MCP thread so a wedged transport never
        // hangs process exit.
        let _ = minecraft_mcp_rs::bot::spawn::join_with_timeout(mcp_handle, Duration::from_secs(3));
        return;
    }

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
        minecraft_mcp_rs::ui::i18n::tr(minecraft_mcp_rs::ui::i18n::TextKey::AppTitle),
        native_options,
        Box::new(move |cc| {
            // Install the platform-default CJK system font so Simplified
            // Chinese characters render correctly (egui's bundled fonts
            // are Latin-only).  Falls back to the default with a warning
            // if no CJK font is installed on the host.
            minecraft_mcp_rs::ui::fonts::install_system_cjk_fonts(&cc.egui_ctx);
            // Re-sync the i18n language from the persisted config in case
            // anything changed between the early `set()` call above and
            // the egui closure firing.
            let lang = state_for_egui.read_config().language;
            minecraft_mcp_rs::ui::i18n::set(lang);
            Ok(Box::new(MinecraftApp::new(
                state_for_egui,
                sender_for_egui,
                receiver_for_egui,
                mcp_handle,
            )))
        }),
    )
    .expect("Failed to start egui");
}
