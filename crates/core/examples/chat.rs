//! Throwaway manual smoke test: talk to a real Anthropic model through `CoreApp::send_message`.
//!
//! Not part of the milestone plan — M6 (CLI) and M7 (desktop) are where this wiring really
//! belongs. This just proves the M3 orchestrator works end-to-end against a live provider
//! before either of those exists. No plugins are registered, so the model only has the four
//! core tools (`core_log_weight`, `core_remember`, `core_forget`, `core_delete_entry`).
//!
//! Usage:
//!   ANTHROPIC_API_KEY=sk-ant-... cargo run -p lifeagent-core --example chat
//!   ANTHROPIC_API_KEY=sk-ant-... ANTHROPIC_MODEL=claude-opus-5-5 cargo run -p lifeagent-core --example chat
//!
//! The key lives only in an in-memory `MemorySecretStore` for this process — it is never
//! written to disk. Conversation history persists across runs in a scratch data dir under the
//! OS temp directory (printed on startup); delete it to start fresh.

use std::io::{self, BufRead, Write};
use std::sync::Arc;

use chrono::Utc;
use lifeagent_core::app::CoreAppBuilder;
use lifeagent_core::db;
use lifeagent_core::secrets::{MemorySecretStore, SecretStore};
use lifeagent_core::settings::Settings;
use lifeagent_llm::router::{ModelsConfig, ProviderConfig, ProviderKind, RoleBinding};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let api_key = std::env::var("ANTHROPIC_API_KEY")
        .map_err(|_| "set ANTHROPIC_API_KEY to your Anthropic API key")?;
    let model = std::env::var("ANTHROPIC_MODEL").unwrap_or_else(|_| "claude-sonnet-5".into());

    let data_dir = std::env::temp_dir().join("lifeagent-throwaway-chat");
    println!("(scratch data dir: {})", data_dir.display());

    let binding = RoleBinding {
        provider_id: "anthropic".into(),
        model: model.clone(),
        temperature: None,
        effort: None,
    };
    let config = ModelsConfig {
        providers: vec![ProviderConfig {
            id: "anthropic".into(),
            kind: ProviderKind::Anthropic,
            base_url: "https://api.anthropic.com".into(),
        }],
        parser: binding.clone(),
        planner: binding.clone(),
        conversation: binding,
    };

    // `CoreAppBuilder::build` reads `settings.models()` once, at build time, so the config has
    // to already be in the database before the builder opens it — write it through a
    // throwaway pool first, then let the builder reopen the same file.
    {
        let db = db::open(&data_dir).await?;
        Settings::new(db).set_models(&config, Utc::now()).await?;
    }

    let secrets = Arc::new(MemorySecretStore::new());
    secrets.set("anthropic", &api_key)?;

    let app = CoreAppBuilder::new(data_dir)
        .secrets(secrets)
        .build()
        .await?;

    println!(
        "Connected to Anthropic model {model:?}. Type a message (Ctrl+D or `exit` to quit).\n"
    );

    let stdin = io::stdin();
    let mut stdout = io::stdout();
    loop {
        print!("> ");
        stdout.flush()?;

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

        match app.send_message(text).await {
            Ok(turn) => {
                println!("{}", turn.reply);
                for activity in &turn.activities {
                    println!("  · {activity}");
                }
            }
            Err(e) => eprintln!("error: {e}"),
        }
        println!();
    }

    Ok(())
}
