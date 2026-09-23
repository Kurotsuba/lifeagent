//! `lifeagent-cli`: developer harness (chat, plan, inspect data).
//!
//! The command tree below matches `docs/PHASE1_SPEC.md` §11. Wiring each subcommand to
//! `CoreApp` is M6 work; this M0 skeleton only establishes the CLI surface so
//! `cargo run -p lifeagent-cli -- --help` is meaningful from the first commit.

use clap::{Parser, Subcommand};

/// LifeAgent developer CLI.
#[derive(Parser)]
#[command(name = "lifeagent-cli", version, about)]
struct Cli {
    /// Override the data directory (used for development and tests).
    #[arg(long, global = true)]
    data_dir: Option<std::path::PathBuf>,

    /// Use a mock chat model that echoes input, for smoke-testing plumbing only.
    #[arg(long, global = true)]
    mock: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Interactive REPL; prints reply and activities.
    Chat,
    /// One conversation turn.
    Say { text: String },
    /// Print `TodayView` as pretty JSON.
    Today,
    /// Plan generation, inspection and revision.
    Plan {
        #[command(subcommand)]
        action: PlanAction,
    },
    /// Food table inspection.
    Foods {
        #[command(subcommand)]
        action: FoodsAction,
    },
    /// Profile inspection and update.
    Profile {
        #[command(subcommand)]
        action: ProfileAction,
    },
    /// Model configuration.
    Models {
        #[command(subcommand)]
        action: ModelsAction,
    },
    /// Print `NutritionTargets` for today.
    Targets,
}

#[derive(Subcommand)]
enum PlanAction {
    Generate {
        #[arg(long)]
        date: Option<String>,
    },
    Show {
        #[arg(long)]
        date: Option<String>,
    },
    Revise {
        instruction: String,
        #[arg(long)]
        date: Option<String>,
    },
}

#[derive(Subcommand)]
enum FoodsAction {
    List {
        #[arg(long)]
        limit: Option<u32>,
    },
}

#[derive(Subcommand)]
enum ProfileAction {
    Show,
    SetJson { file: std::path::PathBuf },
}

#[derive(Subcommand)]
enum ModelsAction {
    SetJson {
        file: std::path::PathBuf,
    },
    /// Reads the key from stdin, hidden.
    SetKey {
        provider_id: String,
    },
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    match cli.command {
        Command::Chat
        | Command::Say { .. }
        | Command::Today
        | Command::Plan { .. }
        | Command::Foods { .. }
        | Command::Profile { .. }
        | Command::Models { .. }
        | Command::Targets => {
            eprintln!("lifeagent-cli: command surface only (M0 skeleton); wired to CoreApp in M6.");
        }
    }

    Ok(())
}
