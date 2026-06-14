mod block_data;
mod config;
pub mod logging;
pub mod types;

fn main() {
    logging::init_logging();

    tracing::info!("Minecraft MCP server starting");
}
