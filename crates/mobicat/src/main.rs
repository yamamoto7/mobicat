mod config;
mod run;
mod synth;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};

/// Android and iOS app logs in one terminal UI.
///
/// Follows the app across restarts, shows only the app's own logs by default,
/// and highlights crashes. Settings can live in .mobicat.toml (see `mobicat config`).
#[derive(Parser)]
#[command(name = "mobicat", version, args_conflicts_with_subcommands = true)]
pub struct Cli {
    /// Package name / bundle ID. Used for both platforms unless --android / --ios is given.
    /// Without one, mobicat asks (or uses `app` from .mobicat.toml).
    pub app: Option<String>,

    /// Android package name, when it differs from APP.
    #[arg(long, value_name = "PACKAGE")]
    pub android: Option<String>,

    /// iOS bundle ID, when it differs from APP.
    #[arg(long, value_name = "BUNDLE_ID")]
    pub ios: Option<String>,

    /// Only use these devices (Android serial or simulator UDID). Repeatable.
    #[arg(long = "device", short = 'd', value_name = "ID")]
    pub devices: Vec<String>,

    /// Filter query, e.g. 'tag:OkHttp level:warn -tag:Chatty' (see `/` in the TUI).
    #[arg(long, short = 'q', value_name = "QUERY")]
    pub query: Option<String>,

    /// Apply a saved filter from the config file.
    #[arg(long, short = 'p', value_name = "NAME")]
    pub profile: Option<String>,

    /// Print NDJSON instead of opening the TUI.
    #[arg(long, conflicts_with = "plain")]
    pub json: bool,

    /// Print one line per entry instead of opening the TUI.
    #[arg(long)]
    pub plain: bool,

    /// Show every line of the app's processes, not only the app's own logs.
    #[arg(long)]
    pub all: bool,

    /// Restart the app so its startup is captured. On iOS this also captures
    /// stdout (print(), Capacitor's console.log).
    #[arg(long)]
    pub launch: bool,

    /// Start with new lines only (default: include the last 5 minutes on Android).
    #[arg(long)]
    pub clear: bool,

    /// Read a saved log (logcat text or `log stream` ndjson). `-` reads stdin.
    #[arg(long, value_name = "PATH", conflicts_with = "launch")]
    pub file: Option<String>,

    /// When to use color in --plain output. Also honors NO_COLOR.
    #[arg(long, value_enum, default_value_t = ColorChoice::Auto)]
    pub color: ColorChoice,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum ColorChoice {
    Auto,
    Always,
    Never,
}

#[derive(Subcommand)]
pub enum Command {
    /// List connected Android devices and booted iOS simulators.
    Devices,
    /// Show which config files are used and the resulting settings.
    Config,
    /// Developer tools.
    #[command(hide = true, subcommand)]
    Dev(DevCommand),
}

#[derive(Subcommand)]
pub enum DevCommand {
    /// Run the TUI on a synthetic log stream (performance spike).
    Synth(synth::SynthArgs),
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Dev(DevCommand::Synth(args))) => synth::run(args),
        Some(Command::Devices) => run::devices(),
        Some(Command::Config) => run::show_config(),
        None => run::logs(&cli),
    }
}

impl Cli {
    pub fn file_path(&self) -> Option<Option<PathBuf>> {
        self.file
            .as_ref()
            .map(|f| (f != "-").then(|| PathBuf::from(f)))
    }
}
