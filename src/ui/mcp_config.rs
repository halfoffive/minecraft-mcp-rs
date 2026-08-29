//! MCP Config panel: JSON snippet users can copy into their MCP client.
//!
//! Renders a read-only, pretty-printed JSON configuration whose shape
//! depends on the selected [`McpTransport`]:
//!
//! - `Http` — emits a `url` block, plus a `headers.Authorization` block
//!   when HTTP auth is enabled (`mcp_auth_enabled`), for remote HTTP
//!   clients.
//! - `Stdio` — emits the classic `command` + `args` block for local
//!   subprocess clients (Claude Desktop / Cursor).
//!
//! The JSON is cached (L-19) and rebuilt only when a JSON-affecting
//! [`EditConfig`] value changes; the npx/bunx snippets and the executable
//! path are constants resolved once via [`LazyLock`].

use egui::{FontId, TextEdit, Ui};
use std::sync::LazyLock;

use crate::config::McpTransport;
use crate::i18n::{self, TextKey};
use crate::ui::app::EditConfig;

/// The absolute path of the running executable, resolved ONCE (L-19).
///
/// Previously the stdio JSON called [`std::env::current_exe`] on every
/// frame. Falls back to `"minecraft-mcp-rs"` when the platform cannot
/// resolve the path.
static EXE_PATH: LazyLock<String> = LazyLock::new(|| {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "minecraft-mcp-rs".to_owned())
});

/// The npx launcher JSON — built once on first use and pinned to
/// `CARGO_PKG_VERSION` for the process lifetime (L-19). It is a
/// [`LazyLock`], not a compile-time constant.
static NPX_JSON: LazyLock<String> = LazyLock::new(build_npx_config_json);

/// The bunx launcher JSON — built once on first use and pinned to
/// `CARGO_PKG_VERSION` for the process lifetime (L-19). See
/// [`NPX_JSON`].
static BUNX_JSON: LazyLock<String> = LazyLock::new(build_bunx_config_json);

/// Cache for the MCP Config panel's pretty-printed JSON (L-19).
///
/// Rebuilding the JSON every frame was a per-frame `serde_json::to_string_pretty`
/// call; combined with the per-frame [`std::env::current_exe`] (now a
/// [`LazyLock`]) this was pure waste when nothing changed. The cache keys on
/// the JSON-affecting [`EditConfig`] inputs and rebuilds only when one of
/// them differs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpConfigCache {
    transport: McpTransport,
    mcp_address: String,
    mcp_port: u16,
    mcp_token: String,
    mcp_auth_enabled: bool,
    /// Pretty-printed main JSON for the cached inputs.
    json: String,
    /// Number of rebuilds performed via [`McpConfigCache::get`] (0 right
    /// after construction — the initial build is not counted). Test
    /// diagnostic proving cache hits do not rebuild.
    rebuilds: u64,
}

impl McpConfigCache {
    /// Build the cache fresh for `edit` (one JSON build).
    pub fn new(edit: &EditConfig) -> Self {
        Self::build(edit, 0)
    }

    fn build(edit: &EditConfig, rebuilds: u64) -> Self {
        Self {
            transport: edit.mcp_transport,
            mcp_address: edit.mcp_address.clone(),
            mcp_port: edit.mcp_port,
            mcp_token: edit.mcp_token.clone(),
            mcp_auth_enabled: edit.mcp_auth_enabled,
            json: build_mcp_config_json(edit),
            rebuilds,
        }
    }

    /// Return the JSON for `edit`, rebuilding only when a JSON-affecting
    /// input changed. The npx/bunx snippets are constants and never
    /// participate in the rebuild decision.
    pub fn get(&mut self, edit: &EditConfig) -> &str {
        if self.transport != edit.mcp_transport
            || self.mcp_address != edit.mcp_address
            || self.mcp_port != edit.mcp_port
            || self.mcp_token != edit.mcp_token
            || self.mcp_auth_enabled != edit.mcp_auth_enabled
        {
            *self = Self::build(edit, self.rebuilds + 1);
        }
        &self.json
    }

    /// Number of rebuilds performed by [`McpConfigCache::get`].
    pub fn rebuilds(&self) -> u64 {
        self.rebuilds
    }
}

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
///       "url": "http://<mcp_address>:<mcp_port>/mcp"
///     }
///   }
/// }
/// ```
///
/// The `headers.Authorization` block is appended to the `minecraft` entry
/// only when [`EditConfig::mcp_auth_enabled`] is true:
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
///       "args": ["--stdio"]
///     }
///   }
/// }
/// ```
///
/// The `<absolute_path_to_executable>` is obtained from
/// [`std::env::current_exe`] once into a [`LazyLock`] (L-19).  If that
/// fails (e.g. the platform cannot resolve the exe path), the string
/// `"minecraft-mcp-rs"` is used as a fallback.
pub fn mcp_config_panel(ui: &mut Ui, edit: &EditConfig, cache: &mut McpConfigCache) {
    let json_text = cache.get(edit);

    // ── Pending-edits hint ─────────────────────────────────────
    // The JSON is generated from the edit buffers, so while any field has
    // un-applied local edits it can differ from the live config until the
    // user clicks Connect (2026-08-29 review — previously silent).
    if edit.dirty.any() {
        ui.label(
            egui::RichText::new(i18n::tr(TextKey::ConfigPendingHint))
                .color(egui::Color32::from_rgb(220, 160, 60))
                .small(),
        );
    }

    // ── Copy button + hint ─────────────────────────────────────
    ui.horizontal(|ui| {
        if ui.button(i18n::tr(TextKey::Copy)).clicked() {
            ui.ctx().copy_text(json_text.to_owned());
        }
        ui.label(i18n::tr(TextKey::CopyHint));
    });

    // ── Security warning for non-loopback HTTP binds ───────────
    // HTTP transport sends the Bearer token in cleartext (no TLS).
    // Only loopback addresses are safe to bind without TLS — anything
    // else (including `0.0.0.0`, which binds all interfaces) exposes
    // the token to anyone on the network. The predicate is config.rs's
    // loopback check — the same rule validate() applies, so any
    // loopback spelling (e.g. 127.0.0.2) agrees on both sides
    // (2026-08-26 review). An empty address is suppressed like the
    // Settings panel does (2026-08-29 review: the two panels used to
    // disagree on that case).
    if edit.mcp_transport == McpTransport::Http {
        let is_safe = crate::config::is_loopback_bind_address(&edit.mcp_address);
        if !is_safe && !edit.mcp_address.is_empty() {
            ui.label(
                egui::RichText::new(i18n::tr(TextKey::TlsWarning))
                    .color(egui::Color32::from_rgb(220, 80, 80)),
            );
        }
    }

    // ── Read-only JSON display ─────────────────────────────────
    // `interactive(false)` makes the field read-only (no cursor / editing).
    // `desired_width(INFINITY)` stretches it to fill the available width so
    // the full executable path / URL is visible without horizontal scrolling.
    let mut text = json_text.to_owned();
    ui.add(
        TextEdit::multiline(&mut text)
            .font(FontId::monospace(12.0))
            .interactive(false)
            .desired_width(f32::INFINITY),
    );

    // ── npx / bunx variants (Stdio only) ──────────────────────
    // The npx / bunx launchers only make sense for the stdio transport —
    // they spawn the binary as a subprocess, which is exactly what the Stdio
    // JSON block describes. HTTP transport is unaffected. The snippets are
    // compile-time constants (LazyLock, L-19).
    if edit.mcp_transport == McpTransport::Stdio {
        ui.add_space(6.0);
        ui.label(i18n::tr(TextKey::NpxConfig));
        ui.horizontal(|ui| {
            if ui.button(i18n::tr(TextKey::Copy)).clicked() {
                ui.ctx().copy_text(NPX_JSON.clone());
            }
        });
        let mut npx_text = NPX_JSON.as_str().to_owned();
        ui.add(
            TextEdit::multiline(&mut npx_text)
                .font(FontId::monospace(12.0))
                .interactive(false)
                .desired_width(f32::INFINITY),
        );

        // bunx variant — parallel block for users on Bun. `bunx` auto-installs
        // the package without prompting, so this JSON has no `-y` flag.
        ui.add_space(6.0);
        ui.label(i18n::tr(TextKey::BunxConfig));
        ui.horizontal(|ui| {
            if ui.button(i18n::tr(TextKey::Copy)).clicked() {
                ui.ctx().copy_text(BUNX_JSON.clone());
            }
        });
        let mut bunx_text = BUNX_JSON.as_str().to_owned();
        ui.add(
            TextEdit::multiline(&mut bunx_text)
                .font(FontId::monospace(12.0))
                .interactive(false)
                .desired_width(f32::INFINITY),
        );
    }
}

/// Format a BIND host string as a CLIENT-connectable URL host (report P3).
///
/// - IPv6 addresses get square brackets per RFC 3986 (::1 becomes [::1]);
/// - a bind-all address is NOT connectable by a client — 0.0.0.0 (and ::)
///   would make the MCP client target itself and usually fail, so it is
///   rewritten to the loopback address the local client must use
///   (127.0.0.1 / [::1]).
fn format_host_for_url(host: &str) -> String {
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V6(addr)) if addr.is_unspecified() => "[::1]".to_string(),
        Ok(std::net::IpAddr::V4(addr)) if addr.is_unspecified() => "127.0.0.1".to_string(),
        Ok(std::net::IpAddr::V6(_)) => format!("[{host}]"),
        _ => host.to_owned(),
    }
}

/// Build the MCP client config JSON for the given [`EditConfig`].
///
/// Returns a pretty-printed JSON string.  The shape branches on
/// [`EditConfig::mcp_transport`]:
///
/// - [`McpTransport::Http`] — `url` (+ `headers.Authorization` only when
///   [`EditConfig::mcp_auth_enabled`]).
/// - [`McpTransport::Stdio`] — `command` + `args` block (uses the
///   [`EXE_PATH`] cached from [`std::env::current_exe`]).
fn build_mcp_config_json(edit: &EditConfig) -> String {
    let json = match edit.mcp_transport {
        McpTransport::Http => {
            let host = format_host_for_url(&edit.mcp_address);
            let url = format!("http://{host}:{}/mcp", edit.mcp_port);
            // The Authorization header is only meaningful when the server's
            // HTTP auth is actually enabled (`mcp_auth_enabled`). When auth
            // is off the generated config must not leak the (possibly random)
            // token into the clipboard-ready JSON, and clients should not
            // send a bearer header to an unauthenticated endpoint.
            let mut minecraft = serde_json::json!({ "url": url });
            if edit.mcp_auth_enabled {
                minecraft["headers"] =
                    serde_json::json!({ "Authorization": format!("Bearer {}", edit.mcp_token) });
            }
            serde_json::json!({
                "mcpServers": {
                    "minecraft": minecraft
                }
            })
        }
        McpTransport::Stdio => {
            let exe_path = EXE_PATH.as_str();
            // `--stdio` is mandatory: with no args the binary prints help and
            // exits (T1), which would silently kill the MCP subprocess.
            serde_json::json!({
                "mcpServers": {
                    "minecraft": {
                        "command": exe_path,
                        "args": ["--stdio"]
                    }
                }
            })
        }
    };
    serde_json::to_string_pretty(&json).unwrap_or_else(|_| "{}".to_owned())
}

/// The exact npm package specifier for the published binary, e.g.
/// `minecraft-mcp-rs@<version>`.
///
/// Derived from `env!("CARGO_PKG_VERSION")` so the pin can never drift from
/// the Cargo.toml version — the npm packages share that version via
/// `npm/scripts/sync-versions.mjs` on release. We pin an exact version (never
/// the floating `@latest` tag) so MCP clients get a reproducible binary.
fn npm_package_pin() -> String {
    format!("minecraft-mcp-rs@{}", env!("CARGO_PKG_VERSION"))
}

/// Build the npm / npx variant of the MCP client config JSON.
///
/// Launches the published npm package through `npx` — no Rust toolchain
/// needed on the client machine. Requires the package to be published (see
/// `npm/` and the `npm-publish` CI job); the args mirror the flags the
/// binary parses: `--headless --stdio`. The package specifier is pinned to
/// the exact version via [`npm_package_pin`] (drift-proof against Cargo).
fn build_npx_config_json() -> String {
    let json = serde_json::json!({
        "mcpServers": {
            "minecraft": {
                "command": "npx",
                "args": ["-y", npm_package_pin(), "--headless", "--stdio"]
            }
        }
    });
    serde_json::to_string_pretty(&json).unwrap_or_else(|_| "{}".to_owned())
}

/// Build the bunx variant of the MCP client config JSON.
///
/// Same idea as [`build_npx_config_json`] but for users on Bun: `bunx`
/// auto-installs the package without prompting, so no `-y` flag is needed
/// (unlike `npx`). The args still mirror the binary's headless stdio flags,
/// and the package specifier is pinned to the exact version via
/// [`npm_package_pin`].
fn build_bunx_config_json() -> String {
    let json = serde_json::json!({
        "mcpServers": {
            "minecraft": {
                "command": "bunx",
                "args": [npm_package_pin(), "--headless", "--stdio"]
            }
        }
    });
    serde_json::to_string_pretty(&json).unwrap_or_else(|_| "{}".to_owned())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;

    // -- format_host_for_url tests -----------------------------------------

    #[test]
    fn test_format_host_ipv4_no_brackets() {
        assert_eq!(format_host_for_url("127.0.0.1"), "127.0.0.1");
        assert_eq!(format_host_for_url("192.168.1.1"), "192.168.1.1");
        assert_eq!(format_host_for_url("0.0.0.0"), "127.0.0.1");
    }

    #[test]
    fn test_format_host_ipv6_with_brackets() {
        assert_eq!(format_host_for_url("::1"), "[::1]");
        assert_eq!(format_host_for_url("2001:db8::1"), "[2001:db8::1]");
        assert_eq!(format_host_for_url("fe80::1"), "[fe80::1]");
    }

    #[test]
    fn test_format_host_hostname_no_brackets() {
        assert_eq!(format_host_for_url("localhost"), "localhost");
        assert_eq!(format_host_for_url("example.com"), "example.com");
    }

    // -- HTTP transport -----------------------------------------------------

    #[test]
    fn test_mcp_config_http_json_ipv4() {
        let mut edit = EditConfig::from(&AppConfig::default());
        edit.mcp_transport = McpTransport::Http;
        edit.mcp_address = "127.0.0.1".to_string();
        edit.mcp_port = 3000;
        edit.mcp_token = "my-token".to_string();
        edit.mcp_auth_enabled = true;

        let json = build_mcp_config_json(&edit);

        assert!(
            json.contains(r#""url": "http://127.0.0.1:3000/mcp""#),
            "wrong IPv4 url: {json}"
        );
        assert!(
            json.contains("Authorization"),
            "missing Authorization: {json}"
        );
        assert!(json.contains("Bearer"), "missing Bearer: {json}");
        assert!(json.contains("my-token"), "missing token: {json}");
        assert!(
            !json.contains("\"command\""),
            "should not contain command in HTTP mode: {json}"
        );
    }

    #[test]
    fn test_mcp_config_http_json_ipv6() {
        let mut edit = EditConfig::from(&AppConfig::default());
        edit.mcp_transport = McpTransport::Http;
        edit.mcp_address = "::1".to_string();
        edit.mcp_port = 3000;
        edit.mcp_token = "test-token".to_string();
        edit.mcp_auth_enabled = true;

        let json = build_mcp_config_json(&edit);

        assert!(
            json.contains(r#""url": "http://[::1]:3000/mcp""#),
            "wrong IPv6 url (should have brackets): {json}"
        );
    }

    /// Report P3: a bind-all address must be rewritten to the loopback
    /// host for the client URL — http://0.0.0.0:... and http://[::]:...
    /// are not connectable by a local MCP client.
    #[test]
    fn test_mcp_config_http_json_bind_all_rewritten() {
        let mut edit = EditConfig::from(&AppConfig::default());
        edit.mcp_transport = McpTransport::Http;
        edit.mcp_address = "0.0.0.0".to_string();
        edit.mcp_port = 3000;
        edit.mcp_token = "".to_string();
        edit.mcp_auth_enabled = false;

        let json = build_mcp_config_json(&edit);
        assert!(
            json.contains(r#""url": "http://127.0.0.1:3000/mcp""#),
            "0.0.0.0 must be rewritten to 127.0.0.1: {json}"
        );

        edit.mcp_address = "::".to_string();
        let json2 = build_mcp_config_json(&edit);
        assert!(
            json2.contains(r#""url": "http://[::1]:3000/mcp""#),
            "unspecified IPv6 must be rewritten to [::1]: {json2}"
        );
    }

    #[test]
    fn test_mcp_config_http_json_localhost() {
        let mut edit = EditConfig::from(&AppConfig::default());
        edit.mcp_transport = McpTransport::Http;
        edit.mcp_address = "localhost".to_string();
        edit.mcp_port = 8080;
        edit.mcp_token = "".to_string();
        edit.mcp_auth_enabled = true;

        let json = build_mcp_config_json(&edit);

        assert!(
            json.contains(r#""url": "http://localhost:8080/mcp""#),
            "wrong localhost url: {json}"
        );
    }

    #[test]
    fn test_mcp_config_http_json() {
        let mut edit = EditConfig::from(&AppConfig::default());
        edit.mcp_transport = McpTransport::Http;
        edit.mcp_address = "127.0.0.1".to_string();
        edit.mcp_port = 3000;
        edit.mcp_token = "my-token".to_string();
        edit.mcp_auth_enabled = true;

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

    #[test]
    fn test_mcp_config_http_auth_off_no_headers() {
        let mut edit = EditConfig::from(&AppConfig::default());
        edit.mcp_transport = McpTransport::Http;
        edit.mcp_address = "127.0.0.1".to_string();
        edit.mcp_port = 3000;
        edit.mcp_token = "my-token".to_string();
        // `mcp_auth_enabled` stays false (EditConfig::from default) → the
        // generated config must NOT carry an Authorization header.
        assert!(
            !edit.mcp_auth_enabled,
            "precondition: auth should default off"
        );

        let json = build_mcp_config_json(&edit);

        assert!(
            json.contains(r#""url": "http://127.0.0.1:3000/mcp""#),
            "wrong IPv4 url: {json}"
        );
        assert!(
            !json.contains("Authorization"),
            "auth off must not emit Authorization header: {json}"
        );
        assert!(
            !json.contains("Bearer"),
            "auth off must not emit Bearer: {json}"
        );
        assert!(
            !json.contains("headers"),
            "auth off must not emit headers block: {json}"
        );
        assert!(
            !json.contains("my-token"),
            "auth off must not leak the token: {json}"
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
        assert!(
            json.contains("\"--stdio\""),
            "stdio args must run the binary headless (--stdio): {json}"
        );
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

    // -- npx variant ---------------------------------------------------------

    #[test]
    fn test_build_npx_config_json_contents() {
        let json = build_npx_config_json();
        assert!(json.contains("npx"), "missing npx command: {json}");
        assert!(json.contains("-y"), "missing -y flag: {json}");
        assert!(json.contains("minecraft-mcp-rs"), "missing package: {json}");
        assert!(
            json.contains(&format!("minecraft-mcp-rs@{}", env!("CARGO_PKG_VERSION"))),
            "missing version-pinned package: {json}"
        );
        assert!(
            !json.contains("@latest"),
            "must not use @latest, should pin the Cargo version: {json}"
        );
        assert!(json.contains("--headless"), "missing --headless: {json}");
        assert!(json.contains("--stdio"), "missing --stdio: {json}");
        assert!(
            !json.contains("current_exe"),
            "npx config must not reference the local exe path: {json}"
        );
    }

    // -- bunx variant ---------------------------------------------------------

    #[test]
    fn test_build_bunx_config_json_contents() {
        let json = build_bunx_config_json();
        assert!(json.contains("bunx"), "missing bunx command: {json}");
        assert!(
            json.contains(&format!("minecraft-mcp-rs@{}", env!("CARGO_PKG_VERSION"))),
            "missing version-pinned package: {json}"
        );
        assert!(
            !json.contains("@latest"),
            "must not use @latest, should pin the Cargo version: {json}"
        );
        assert!(json.contains("--headless"), "missing --headless: {json}");
        assert!(json.contains("--stdio"), "missing --stdio: {json}");
        assert!(
            !json.contains("\"-y\""),
            "bunx must NOT use -y (it auto-installs without prompting): {json}"
        );
    }

    #[test]
    fn test_package_pin_matches_cargo_version() {
        let pin = npm_package_pin();
        assert_eq!(
            pin,
            format!("minecraft-mcp-rs@{}", env!("CARGO_PKG_VERSION")),
            "pin must match the Cargo.toml version"
        );
        assert!(
            !pin.contains("@latest"),
            "pin must never be the floating @latest tag: {pin}"
        );
    }

    // -- L-19: JSON caching --------------------------------------------------

    /// L-19: unchanged inputs must reuse the cached JSON (no rebuild);
    /// a changed input forces exactly one rebuild. Previously the panel
    /// rebuilt the JSON (and called `std::env::current_exe()`) every frame.
    #[test]
    fn test_mcp_config_json_cached_when_unchanged() {
        let mut edit = EditConfig::from(&AppConfig::default());
        edit.mcp_transport = McpTransport::Http;
        edit.mcp_address = "127.0.0.1".into();
        edit.mcp_port = 3000;
        edit.mcp_token = "tok".into();
        edit.mcp_auth_enabled = true;

        let mut cache = McpConfigCache::new(&edit);
        assert_eq!(cache.rebuilds(), 0, "fresh cache has not rebuilt yet");
        let first = cache.get(&edit).to_owned();
        assert_eq!(cache.rebuilds(), 0, "first get on a fresh cache is a hit");

        let second = cache.get(&edit).to_owned();
        assert_eq!(first, second, "unchanged inputs must reuse the JSON");
        assert_eq!(cache.rebuilds(), 0, "identical reads must not rebuild");

        // A changed input forces a rebuild with the fresh value.
        edit.mcp_port = 4000;
        let third = cache.get(&edit).to_owned();
        assert_ne!(first, third, "changed port must produce new JSON");
        assert!(
            third.contains("4000"),
            "rebuilt JSON reflects the port: {third}"
        );
        assert_eq!(cache.rebuilds(), 1, "exactly one rebuild for one change");
    }
}
