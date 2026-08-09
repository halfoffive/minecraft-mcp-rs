//! Command-line argument parsing for the binary.
//!
//! The binary supports three behaviours:
//! - **Help mode** (no arguments): prints usage to stderr and exits 0.
//! - **GUI mode** (`--gui`): opens the egui desktop window.
//! - **Headless mode** (`--headless`, or `--stdio` alone): no window; the bot
//!   auto-connects and the process exits when the MCP transport closes or on
//!   shutdown.
//!
//! Parsing is manual (no clap or other dependency): only five flags exist
//! (`--headless`, `--stdio`, `--gui`, `--config <path>`, `-h`/`--help`), so a
//! small hand-rolled parser keeps the dependency tree lean.
//!
//! **Never print to stdout from this module** — stdout is the MCP JSON-RPC
//! transport. Help text goes to stderr via [`print_help`].

/// Parsed command-line arguments.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CliArgs {
    /// Run without the egui window: auto-connect the bot on startup and exit
    /// when the MCP transport closes.
    pub headless: bool,
    /// Force the MCP stdio transport regardless of the configured transport
    /// (e.g. when launched by `npx` as an MCP server).
    pub force_stdio: bool,
    /// Open the egui desktop window explicitly.
    pub gui: bool,
    /// Path to a config file to load instead of the default OS config dir.
    pub config_path: Option<std::path::PathBuf>,
    /// Print help and exit.
    pub help: bool,
}

impl CliArgs {
    /// Whether no mode/flag was specified at all (all flags false, no config
    /// path). An empty argument set means the user asked for nothing, which
    /// `resolve_mode` treats as a help request.
    pub fn is_empty(&self) -> bool {
        !self.headless && !self.force_stdio && !self.gui && !self.help && self.config_path.is_none()
    }
}

/// Which behaviour the binary should take based on the parsed arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunMode {
    /// Print help to stderr and exit 0 (no arguments, or `--help`).
    Help,
    /// Open the egui desktop window (`--gui`).
    Gui,
    /// Run without a window (`--headless`, or `--stdio` alone).
    Headless,
}

/// Resolve the run mode from the parsed arguments.
///
/// Precedence: help wins over everything; otherwise an explicit `--headless`
/// (or `--stdio` *without* `--gui`) forces headless; anything else — including
/// `--config` alone — runs the GUI. `--gui` beats the `--stdio`-alone
/// implication, so `--gui --stdio` still opens the window.
pub fn resolve_mode(args: &CliArgs) -> RunMode {
    if args.help || args.is_empty() {
        RunMode::Help
    } else if args.headless || (args.force_stdio && !args.gui) {
        RunMode::Headless
    } else {
        RunMode::Gui
    }
}

/// Parse command-line arguments (excluding the program name).
///
/// # Errors
///
/// Returns a message describing the first unknown argument or a missing
/// `--config` value. The caller should print it to stderr and exit non-zero.
pub fn parse_cli_args<I: Iterator<Item = String>>(args: I) -> Result<CliArgs, String> {
    let mut out = CliArgs::default();
    let mut args = args.peekable();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--headless" => out.headless = true,
            "--stdio" => out.force_stdio = true,
            "--gui" => out.gui = true,
            "-h" | "--help" => out.help = true,
            "--config" => {
                let value = args.next().ok_or("--config requires a path argument")?;
                if value.starts_with('-') {
                    return Err(format!("--config requires a path argument, got {value:?}"));
                }
                out.config_path = Some(std::path::PathBuf::from(value));
            }
            _ => return Err(format!("unknown argument: {arg} (try --help)")),
        }
    }

    Ok(out)
}

/// Print usage information to stderr (never stdout — stdout is the MCP
/// transport).
pub fn print_help() {
    eprintln!("minecraft-mcp-rs — MCP server that controls a Minecraft bot");
    eprintln!();
    eprintln!("USAGE:");
    eprintln!("    minecraft-mcp-rs [OPTIONS]");
    eprintln!();
    eprintln!("OPTIONS:");
    eprintln!("    --gui              Open the desktop window explicitly");
    eprintln!("    --headless         Run without the desktop window; auto-connect the bot");
    eprintln!("                       and exit when the MCP transport closes");
    eprintln!("    --stdio            Force the MCP stdio transport (overrides the config)");
    eprintln!("    --config <path>    Load the config file at <path> instead of the OS config dir");
    eprintln!("    -h, --help         Print this help and exit");
    eprintln!();
    eprintln!(
        "With no arguments prints this help. Use --gui for the desktop UI; --stdio alone runs a headless stdio server."
    );
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<CliArgs, String> {
        parse_cli_args(args.iter().map(|s| s.to_string()))
    }

    /// No arguments → defaults (UI mode, no help).
    #[test]
    fn test_empty_args_returns_defaults() {
        assert_eq!(parse(&[]).unwrap(), CliArgs::default());
    }

    /// `--headless` alone enables headless mode.
    #[test]
    fn test_headless_flag() {
        let args = parse(&["--headless"]).unwrap();
        assert!(args.headless);
        assert!(!args.force_stdio);
        assert!(!args.help);
        assert!(args.config_path.is_none());
    }

    /// `--stdio` alone forces the stdio transport.
    #[test]
    fn test_stdio_flag() {
        let args = parse(&["--stdio"]).unwrap();
        assert!(args.force_stdio);
        assert!(!args.headless);
    }

    /// `--config` captures its value as a path.
    #[test]
    fn test_config_with_value() {
        let args = parse(&["--config", "/tmp/my-config.json"]).unwrap();
        assert_eq!(
            args.config_path,
            Some(std::path::PathBuf::from("/tmp/my-config.json"))
        );
    }

    /// `--config` at the end of the args with no value is an error.
    #[test]
    fn test_config_missing_value_errors() {
        let err = parse(&["--config"]).unwrap_err();
        assert!(
            err.contains("--config requires a path argument"),
            "got: {err}"
        );
    }

    /// `--config` followed by another flag is treated as a missing value.
    #[test]
    fn test_config_followed_by_flag_errors() {
        let err = parse(&["--config", "--headless"]).unwrap_err();
        assert!(
            err.contains("--config requires a path argument"),
            "got: {err}"
        );
    }

    /// `-h` and `--help` both set the help flag.
    #[test]
    fn test_help_flags() {
        assert!(parse(&["-h"]).unwrap().help);
        assert!(parse(&["--help"]).unwrap().help);
    }

    /// Unknown arguments produce an error naming the argument.
    #[test]
    fn test_unknown_argument_errors() {
        let err = parse(&["--bogus"]).unwrap_err();
        assert!(err.contains("unknown argument: --bogus"), "got: {err}");
    }

    /// All flags combined parse together.
    #[test]
    fn test_all_flags_combined() {
        let args = parse(&["--headless", "--stdio", "--config", "cfg.json", "--help"]).unwrap();
        assert!(args.headless);
        assert!(args.force_stdio);
        assert!(args.help);
        assert_eq!(args.config_path, Some(std::path::PathBuf::from("cfg.json")));
    }

    /// Repeated flags are idempotent.
    #[test]
    fn test_repeated_flags() {
        let args = parse(&["--headless", "--headless", "--stdio"]).unwrap();
        assert!(args.headless);
        assert!(args.force_stdio);
    }

    /// `--gui` alone enables the GUI mode.
    #[test]
    fn test_gui_flag() {
        let args = parse(&["--gui"]).unwrap();
        assert!(args.gui);
        assert!(!args.headless);
        assert!(!args.force_stdio);
        assert!(!args.help);
    }

    /// No arguments → help mode (no longer starts the UI).
    #[test]
    fn test_resolve_mode_no_args_help() {
        assert_eq!(resolve_mode(&CliArgs::default()), RunMode::Help);
    }

    /// `--help` always wins over every other flag.
    #[test]
    fn test_resolve_mode_help_wins() {
        let args = parse(&["--help"]).unwrap();
        assert_eq!(resolve_mode(&args), RunMode::Help);
    }

    /// `--stdio` alone runs a headless stdio server.
    #[test]
    fn test_resolve_mode_stdio_alone_headless() {
        let args = parse(&["--stdio"]).unwrap();
        assert_eq!(resolve_mode(&args), RunMode::Headless);
    }

    /// `--gui` + `--stdio` → GUI (stdio alone is headless, but an explicit
    /// `--gui` overrides that implication).
    #[test]
    fn test_resolve_mode_gui_stdio() {
        let args = parse(&["--gui", "--stdio"]).unwrap();
        assert_eq!(resolve_mode(&args), RunMode::Gui);
    }

    /// `--headless` beats `--gui`.
    #[test]
    fn test_resolve_mode_headless_beats_gui() {
        let args = parse(&["--gui", "--headless"]).unwrap();
        assert_eq!(resolve_mode(&args), RunMode::Headless);
    }

    /// `--config` alone → GUI (a config path does not imply headless).
    #[test]
    fn test_resolve_mode_config_alone_gui() {
        let args = parse(&["--config", "cfg.json"]).unwrap();
        assert_eq!(resolve_mode(&args), RunMode::Gui);
    }

    /// `is_empty`: no flags → true; any single flag → false.
    #[test]
    fn test_is_empty() {
        assert!(CliArgs::default().is_empty());
        assert!(!parse(&["--headless"]).unwrap().is_empty());
        assert!(!parse(&["--stdio"]).unwrap().is_empty());
        assert!(!parse(&["--gui"]).unwrap().is_empty());
        assert!(!parse(&["--help"]).unwrap().is_empty());
        assert!(!parse(&["--config", "x"]).unwrap().is_empty());
    }
}
