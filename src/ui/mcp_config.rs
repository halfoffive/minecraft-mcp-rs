//! MCP Config panel: JSON snippet users can copy into their MCP client.
//!
//! Renders a read-only, pretty-printed JSON configuration containing the
//! absolute path to the current executable.  Users paste this into their
//! MCP client config (Claude Desktop / Cursor) to register the server.

use egui::{FontId, TextEdit, Ui};

/// Render the MCP Config panel.
///
/// Builds a JSON object of the form:
///
/// ```json
/// {
///   "mcpServers": {
///     "minecraft": {
///       "command": "<absolute_path_to_executable>",
///       "args": []
///     }
///   }
/// }
/// ```
///
/// The `<absolute_path_to_executable>` is obtained from
/// [`std::env::current_exe`].  If that fails (e.g. the platform cannot
/// resolve the exe path), the string `"minecraft-mcp-rs"` is used as a
/// fallback.  The JSON is displayed read-only in a monospace text box, and
/// a **Copy** button writes it to the system clipboard via
/// [`egui::Context::copy_text`].
pub fn mcp_config_panel(ui: &mut Ui) {
    // ── Build the JSON config ──────────────────────────────────
    let exe_path = std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "minecraft-mcp-rs".to_owned());

    let json = serde_json::json!({
        "mcpServers": {
            "minecraft": {
                "command": exe_path,
                "args": []
            }
        }
    });

    let json_text = serde_json::to_string_pretty(&json).unwrap_or_else(|_| "{}".to_owned());

    // ── Copy button + hint ─────────────────────────────────────
    ui.horizontal(|ui| {
        if ui.button("Copy").clicked() {
            ui.ctx().copy_text(json_text.clone());
        }
        ui.label("Copy this JSON into your MCP client config (Claude Desktop / Cursor):");
    });

    // ── Read-only JSON display ─────────────────────────────────
    // `interactive(false)` makes the field read-only (no cursor / editing).
    // `desired_width(INFINITY)` stretches it to fill the available width so
    // the full executable path is visible without horizontal scrolling.
    let mut text = json_text;
    ui.add(
        TextEdit::multiline(&mut text)
            .font(FontId::monospace(12.0))
            .interactive(false)
            .desired_width(f32::INFINITY),
    );
}
