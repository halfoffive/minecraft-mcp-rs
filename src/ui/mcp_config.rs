//! MCP Config panel: JSON snippet users can copy into their MCP client.
//!
//! Renders a read-only, pretty-printed JSON configuration whose shape
//! depends on the selected [`McpTransport`](crate::config::McpTransport):
//!
//! - `Http` — emits a `url` + `headers.Authorization` block for remote
//!   HTTP clients.
//! - `Stdio` — emits the classic `command` + `args` block for local
//!   subprocess clients (Claude Desktop / Cursor).
//!
//! The JSON is regenerated every frame from the current [`EditConfig`]
//! values, so edits in the Settings panel (token, port, transport) are
//! reflected immediately in the MCP Config panel.

use egui::{FontId, TextEdit, Ui};

use crate::config::McpTransport;
use crate::ui::app::EditConfig;

/// Render the MCP Config panel.
///
/// Builds a JSON object based on [`EditConfig::mcp_transport`] and
/// displays it read-only in a monospace text box.  A **Copy** button
/// writes the JSON to the system clipboard via [`egui::Context::copy_text`].
///
/// - When transport is [`McpTransport::Http`], the JSON has the form:
///
/// ```json
/// {
///   "mcpServers": {
///     "minecraft": {
///       "url": "http://<mcp_address>:<mcp_port>/mcp",
///       "headers": {
///         "Authorization": "Bearer <mcp_token>"
///       }
///     }
///   }
/// }
/// ```
///
/// - When transport is [`McpTransport::Stdio`], the JSON has the form:
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
/// fallback.
pub fn mcp_config_panel(ui: &mut Ui, edit: &EditConfig) {
    let json_text = build_mcp_config_json(edit);

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
    // the full executable path / URL is visible without horizontal scrolling.
    let mut text = json_text;
    ui.add(
        TextEdit::multiline(&mut text)
            .font(FontId::monospace(12.0))
            .interactive(false)
            .desired_width(f32::INFINITY),
    );
}

/// Build the MCP client config JSON for the given [`EditConfig`].
///
/// Returns a pretty-printed JSON string.  The shape branches on
/// [`EditConfig::mcp_transport`]:
///
/// - [`McpTransport::Http`] — `url` + `headers.Authorization` block.
/// - [`McpTransport::Stdio`] — `command` + `args` block (uses
///   [`std::env::current_exe`] for the executable path).
fn build_mcp_config_json(edit: &EditConfig) -> String {
    let json = match edit.mcp_transport {
        McpTransport::Http => {
            let url = format!("http://{}:{}/mcp", edit.mcp_address, edit.mcp_port);
            serde_json::json!({
                "mcpServers": {
                    "minecraft": {
                        "url": url,
                        "headers": {
                            "Authorization": format!("Bearer {}", edit.mcp_token)
                        }
                    }
                }
            })
        }
        McpTransport::Stdio => {
            let exe_path = std::env::current_exe()
                .ok()
                .and_then(|p| p.to_str().map(|s| s.to_string()))
                .unwrap_or_else(|| "minecraft-mcp-rs".to_owned());
            serde_json::json!({
                "mcpServers": {
                    "minecraft": {
                        "command": exe_path,
                        "args": []
                    }
                }
            })
        }
    };
    serde_json::to_string_pretty(&json).unwrap_or_else(|_| "{}".to_owned())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;

    // -- HTTP transport -----------------------------------------------------

    #[test]
    fn test_mcp_config_http_json() {
        let mut edit = EditConfig::from(&AppConfig::default());
        edit.mcp_transport = McpTransport::Http;
        edit.mcp_address = "127.0.0.1".to_string();
        edit.mcp_port = 3000;
        edit.mcp_token = "my-token".to_string();

        let json = build_mcp_config_json(&edit);

        assert!(json.contains("url"), "missing url: {json}");
        assert!(
            json.contains("Authorization"),
            "missing Authorization: {json}"
        );
        assert!(json.contains("Bearer"), "missing Bearer: {json}");
        assert!(json.contains("3000"), "missing port: {json}");
        assert!(json.contains("my-token"), "missing token: {json}");
        // Stdio-only keys should not appear in HTTP mode.
        assert!(
            !json.contains("\"command\""),
            "should not contain command in HTTP mode: {json}"
        );
    }

    // -- Stdio transport ----------------------------------------------------

    #[test]
    fn test_mcp_config_stdio_json() {
        let mut edit = EditConfig::from(&AppConfig::default());
        edit.mcp_transport = McpTransport::Stdio;

        let json = build_mcp_config_json(&edit);

        assert!(json.contains("command"), "missing command: {json}");
        assert!(json.contains("args"), "missing args: {json}");
        // HTTP-only keys should not appear in Stdio mode.
        assert!(
            !json.contains("\"url\""),
            "should not contain url in Stdio mode: {json}"
        );
        assert!(
            !json.contains("Authorization"),
            "should not contain Authorization in Stdio mode: {json}"
        );
        assert!(
            !json.contains("Bearer"),
            "should not contain Bearer in Stdio mode: {json}"
        );
    }
}
