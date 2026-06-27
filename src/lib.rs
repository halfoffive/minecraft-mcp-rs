//! Minecraft MCP Server — core library
//!
//! Architecture: types → logic → state → bot → mcp → ui
//! Module structure maps to the crate's layered design:
//! - Top-level: foundational types, error handling, config, utilities
//! - `bot`: Minecraft bot lifecycle, event handling, command execution
//! - `mcp`: Model Context Protocol server and tool implementations
//! - `ui`: Desktop UI (status, settings, app shell)

pub mod block_data;
pub mod channel;
pub mod command_validate;
pub mod compound_ops;
pub mod config;
pub mod error;
pub mod logging;
pub mod mining_calc;
pub mod snapshot;
pub mod state;
pub mod tool_select;
pub mod types;
pub mod bot {
    pub mod commands;
    pub mod connection;
    pub mod events;
    pub mod ops;
    pub mod snapshot_updater;
}
pub mod mcp {
    pub mod render;
    pub mod server;
    pub mod tools_act;
    pub mod tools_block;
    pub mod tools_chat;
    pub mod tools_combat;
    pub mod tools_container;
    pub mod tools_item;
    pub mod tools_movement;
    pub mod tools_query;
}
pub mod ui {
    pub mod app;
    pub mod fonts;
    pub mod i18n;
    pub mod mcp_config;
    pub mod settings;
    pub mod status;
}
