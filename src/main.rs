mod block_data;
mod config;
pub mod logging;
pub mod types;

use std::sync::Arc;

use minecraft_mcp_rs::channel;
use minecraft_mcp_rs::config::AppConfig;
use minecraft_mcp_rs::mcp::server::serve_stdio;
use minecraft_mcp_rs::state::SharedState;

#[tokio::main]
async fn main() {
    logging::init_logging();

    tracing::info!("Minecraft MCP server starting");

    let config = AppConfig::default();
    let state = Arc::new(SharedState::new(config));
    let (sender, _receiver) = channel::create_command_channel(64);

    serve_stdio(state, sender).await;

    tracing::info!("MCP server exited");
}
