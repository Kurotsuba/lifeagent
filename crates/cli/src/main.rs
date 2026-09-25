//! `lifeagent-cli`: developer harness (chat, plan, inspect data).
//!
//! The command tree matches `docs/PHASE1_SPEC.md` §11. This crate is a composition root
//! (`CLAUDE.md`): it registers the diet plugin and builds `CoreApp`, since `lifeagent-core`
//! never depends on a plugin crate itself.

mod mock;
mod output;
mod paths;

use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::{NaiveDate, Utc};
use clap::{Parser, Subcommand};
use lifeagent_core::app::{CoreApp, CoreAppBuilder};
use lifeagent_core::plan::PlanTrigger;
use lifeagent_core::profile::Profile;
use lifeagent_core::secrets::{KeyringSecretStore, SecretStore};
use lifeagent_llm::mock::MockChatModel;
use lifeagent_llm::router::ModelsConfig;
use lifeagent_plugin_diet::foods::FoodTable;
use lifeagent_plugin_diet::{intake, DietPlugin};

use output::{print_json, print_turn};

/// `foods list`'s default `--limit` when none is given.
const DEFAULT_FOODS_LIMIT: u32 = 50;

/// LifeAgent developer CLI.
#[derive(Parser)]
#[command(name = "lifeagent-cli", version, about)]
struct Cli {
    /// Override the data directory (used for development and tests).
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,

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
    SetJson { file: PathBuf },
}

#[derive(Subcommand)]
enum ModelsAction {
    SetJson {
        file: PathBuf,
    },
    /// Reads the key from stdin, hidden.
    SetKey {
        provider_id: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("LIFEAGENT_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    // Handled before `CoreApp` is ever built: `CoreAppBuilder::build` constructs a real
    // `LlmRouter` as soon as `models set-json` has stored a provider, which fetches that
    // provider's key up front and fails if it isn't in the keyring yet — exactly the gap
    // `set-key` exists to fill. Keys live only in the OS credential store (never the
    // database), so this needs no data dir and no app at all.
    if let Command::Models {
        action: ModelsAction::SetKey { provider_id },
    } = &cli.command
    {
        return set_key(provider_id);
    }

    let data_dir = match cli.data_dir {
        Some(d) => d,
        None => paths::default_data_dir()?,
    };

    // Built up front, even when `--mock` is off, so its handle is available to `chat`/`say`
    // without threading an `Option<Arc<MockChatModel>>` through `CoreAppBuilder`.
    let mock = Arc::new(MockChatModel::new());
    let mut builder = CoreAppBuilder::new(data_dir).plugin(Arc::new(DietPlugin));
    if cli.mock {
        builder = builder.chat_model_override(mock.clone());
    }
    let app = builder.build().await?;

    match cli.command {
        Command::Chat => chat(&app, cli.mock, &mock).await,
        Command::Say { text } => say(&app, &text, cli.mock, &mock).await,
        Command::Today => print_json(&app.today_view().await?),
        Command::Plan { action } => plan(&app, action).await,
        Command::Foods { action } => foods(&app, action).await,
        Command::Profile { action } => profile(&app, action).await,
        Command::Models { action } => models(&app, action).await,
        Command::Targets => targets(&app).await,
    }
}

async fn chat(app: &CoreApp, mock: bool, mock_model: &MockChatModel) -> Result<()> {
    println!("LifeAgent chat. Type a message (Ctrl+D or \"exit\" to quit).\n");
    let stdin = io::stdin();
    loop {
        print!("> ");
        io::stdout().flush()?;

        let mut line = String::new();
        if stdin.lock().read_line(&mut line)? == 0 {
            break; // EOF
        }
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        if text == "exit" {
            break;
        }

        if mock {
            mock::queue_echo(mock_model);
        }
        print_turn(&app.send_message(text).await?);
    }
    Ok(())
}

async fn say(app: &CoreApp, text: &str, mock: bool, mock_model: &MockChatModel) -> Result<()> {
    if mock {
        mock::queue_echo(mock_model);
    }
    print_turn(&app.send_message(text).await?);
    Ok(())
}

async fn plan(app: &CoreApp, action: PlanAction) -> Result<()> {
    match action {
        PlanAction::Generate { date } => {
            let date = resolve_date(app, date).await?;
            print_json(&app.generate_plan(date, PlanTrigger::Manual).await?)
        }
        PlanAction::Show { date } => {
            let date = resolve_date(app, date).await?;
            print_json(&app.get_plan(date).await?)
        }
        PlanAction::Revise { instruction, date } => {
            let date = resolve_date(app, date).await?;
            print_json(&app.revise_plan(date, &instruction).await?)
        }
    }
}

/// `--date` if given (`YYYY-MM-DD`), else tomorrow — the plan a user is about to eat, not
/// today's already-settled one.
async fn resolve_date(app: &CoreApp, date: Option<String>) -> Result<NaiveDate> {
    match date {
        Some(s) => NaiveDate::parse_from_str(&s, "%Y-%m-%d")
            .with_context(|| format!("invalid --date {s:?}; expected YYYY-MM-DD")),
        None => app
            .logical_today()
            .await?
            .succ_opt()
            .context("date out of range"),
    }
}

async fn foods(app: &CoreApp, action: FoodsAction) -> Result<()> {
    match action {
        FoodsAction::List { limit } => {
            let table = FoodTable::new(&app.plugin_context());
            let foods = table.list(limit.unwrap_or(DEFAULT_FOODS_LIMIT)).await?;
            print_json(&foods)
        }
    }
}

async fn profile(app: &CoreApp, action: ProfileAction) -> Result<()> {
    match action {
        ProfileAction::Show => print_json(&app.profile().get().await?),
        ProfileAction::SetJson { file } => {
            let json = std::fs::read_to_string(&file)
                .with_context(|| format!("reading {}", file.display()))?;
            let profile: Profile = serde_json::from_str(&json)
                .with_context(|| format!("parsing {} as a profile", file.display()))?;
            app.profile().set(&profile).await?;
            println!("profile updated");
            Ok(())
        }
    }
}

async fn models(app: &CoreApp, action: ModelsAction) -> Result<()> {
    match action {
        ModelsAction::SetJson { file } => {
            let json = std::fs::read_to_string(&file)
                .with_context(|| format!("reading {}", file.display()))?;
            let config: ModelsConfig = serde_json::from_str(&json)
                .with_context(|| format!("parsing {} as model config", file.display()))?;
            // `CoreApp` exposes no `Clock`; this is a leaf composition root using the real
            // clock directly, same as the throwaway example harnesses it replaces.
            app.settings().set_models(&config, Utc::now()).await?;
            println!("models updated");
            Ok(())
        }
        // Handled in `main`, before `CoreApp` is built — see the comment there.
        ModelsAction::SetKey { .. } => {
            unreachable!("Command::Models{{action: ModelsAction::SetKey}} returns from main before reaching here")
        }
    }
}

/// `models set-key`: talks to the OS credential store directly, bypassing `CoreApp` (see the
/// call site in `main`).
fn set_key(provider_id: &str) -> Result<()> {
    let key = rpassword::prompt_password(format!("API key for {provider_id}: "))
        .context("reading API key from stdin")?;
    let key = key.trim();
    if key.is_empty() {
        anyhow::bail!("API key must not be empty");
    }
    KeyringSecretStore.set(provider_id, key)?;
    println!("key stored for provider {provider_id:?}");
    Ok(())
}

async fn targets(app: &CoreApp) -> Result<()> {
    let ctx = app.plugin_context();
    let date = app.logical_today().await?;
    let computed = intake::targets_for(&ctx, date).await?.context(
        "no targets yet: set a profile (`profile set-json`) and log a weight first (e.g. via `say`)",
    )?;
    print_json(&computed)
}
