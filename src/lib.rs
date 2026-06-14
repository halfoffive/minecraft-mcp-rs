//! Minecraft MCP Server — core library
//!
//! Architecture: types → logic → state → bot → mcp → ui
//! Module structure maps to the crate's layered design:
//! - Top-level: foundational types, error handling, config, utilities
//! - `bot`: Minecraft bot lifecycle, event handling, command execution
//! - `mcp`: Model Context Protocol server and tool implementations
//! - `ui`: Desktop UI (status, settings, app shell)

pub mod types;
pub mod error;
pub mod config;
pub mod block_data;
pub mod logging;
pub mod state;
pub mod channel;
pub mod tool_select;
pub mod mining_calc;
pub mod snapshot;
pub mod command_validate;
pub mod compound_ops;
pub mod bot {
    pub mod connection;
    pub mod events;
    pub mod commands;
    pub mod ops;
    pub mod snapshot_updater;
}
pub mod mcp {
    pub mod server;
    pub mod tools_query;
    pub mod tools_movement;
    pub mod tools_block;
    pub mod tools_item;
    pub mod tools_container;
    pub mod tools_combat;
    pub mod tools_chat;
}
pub mod ui {
    pub mod app;
    pub mod settings;
    pub mod status;
}
