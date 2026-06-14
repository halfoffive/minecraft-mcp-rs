//! Logging infrastructure for the Minecraft MCP server.
//!
//! # Critical Constraint
//!
//! **All logging output MUST go to stderr.**
//!
//! The MCP protocol uses stdin/stdout as its JSON-RPC transport channel.
//! Any bytes written to stdout (including log messages) will corrupt the
//! protocol stream and break communication with the MCP client.
//!
//! [`init_logging`] configures [`tracing_subscriber`] to write exclusively
//! to stderr, and sets sensible per-crate log levels.

use std::sync::Once;

static INIT: Once = Once::new();

/// Initialize the global tracing subscriber.
///
/// Behaviour:
/// - Writes formatted log output to **stderr** only (stdout is the MCP channel).
/// - Filter: `minecraft_mcp_rs=debug, azalea=warn` (other crates default to error).
/// - Safe to call multiple times — only the first call takes effect.
pub fn init_logging() {
    INIT.call_once(|| {
        tracing_subscriber::fmt()
            .with_writer(std::io::stderr)
            .with_env_filter("minecraft_mcp_rs=debug,azalea=warn")
            .init();
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    /// A [`Write`]r that captures every byte into a shared [`Vec<u8>`] buffer,
    /// allowing test assertions on the serialised log output.
    #[derive(Clone)]
    struct CapturingWriter {
        buffer: Arc<Mutex<Vec<u8>>>,
    }

    impl Write for CapturingWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.buffer.lock().unwrap().write(buf)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.buffer.lock().unwrap().flush()
        }
    }

    /// Verify that a [`tracing_subscriber`] configured identically to
    /// [`init_logging`] (fmt + env-filter) correctly routes events to the
    /// configured writer.
    #[test]
    fn test_logging_configuration() {
        let buffer: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let buf = buffer.clone();

        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || CapturingWriter { buffer: buf.clone() })
            .with_env_filter("minecraft_mcp_rs=debug")
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            tracing::info!("logging module test message");
        });

        let output = buffer.lock().unwrap();
        let output_str =
            String::from_utf8(output.clone()).expect("Tracing output is valid UTF-8");
        assert!(
            output_str.contains("logging module test message"),
            "Tracing output should contain the logged message, got: {output_str}"
        );
    }

    /// **End-to-end stderr verification.**
    ///
    /// Spawns a subprocess that calls [`init_logging`] and emits a trace
    /// event, then asserts the event appears *only* on stderr — proving
    /// the production [`init_logging`] function does not pollute stdout.
    #[test]
    fn test_logging_goes_to_stderr() {
        // ── subprocess branch ──────────────────────────────────────────
        if std::env::var("__MCP_LOG_STDERR_TEST").is_ok() {
            init_logging();
            tracing::info!("mcp stderr verification test message");
            return;
        }

        // ── orchestrator branch ────────────────────────────────────────
        let exe =
            std::env::current_exe().expect("Cannot determine test-binary path");
        let output = std::process::Command::new(&exe)
            .env("__MCP_LOG_STDERR_TEST", "1")
            .arg("test_logging_goes_to_stderr")
            .arg("--nocapture")
            .output()
            .expect("Failed to spawn test subprocess");

        assert!(
            output.status.success(),
            "Subprocess exited with: {:?}",
            output.status
        );

        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(
            stderr.contains("mcp stderr verification test message"),
            "Log output MUST appear on stderr.\n--- stderr ---\n{stderr}\n--- stdout ---\n{stdout}",
            stdout = String::from_utf8_lossy(&output.stdout),
        );

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            !stdout.contains("mcp stderr verification test message"),
            "Log output MUST NOT appear on stdout (MCP channel).\n--- stdout ---\n{stdout}"
        );
    }
}
