//! Throwaway manual harness: the whole app so far (chat, diet plugin, planning, scheduler
//! tick) against a real Anthropic model, from a terminal. Not a milestone deliverable; the
//! M6 CLI replaces it.
//!
//!   ANTHROPIC_API_KEY=sk-ant-... cargo run -p lifeagent-plugin-diet --example try
//!
//! Optional: ANTHROPIC_MODEL (default claude-sonnet-5, used for every role).
//! Plain lines go to the assistant; lines starting with `/` are commands (`/help`).
//! The key stays in memory only. Data persists in a scratch dir (printed at startup);
//! delete it to start over.

use std::io::{self, BufRead, Write};
use std::sync::Arc;

use chrono::{NaiveDate, Utc};
use lifeagent_core::app::{CoreApp, CoreAppBuilder};
use lifeagent_core::conversation::MessageRole;
use lifeagent_core::db;
use lifeagent_core::events::EventSource;
use lifeagent_core::goals::{GoalDirection, WeightGoal};
use lifeagent_core::plan::{PlanTrigger, StoredPlan};
use lifeagent_core::profile::{ActivityLevel, DietaryPattern, Profile, Sex};
use lifeagent_core::scheduler::Scheduler;
use lifeagent_core::secrets::{MemorySecretStore, SecretStore};
use lifeagent_core::settings::Settings;
use lifeagent_llm::router::{ModelsConfig, ProviderConfig, ProviderKind, RoleBinding};
use lifeagent_plugin_diet::model::MealType;
use lifeagent_plugin_diet::DietPlugin;
use serde_json::{json, Value};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Seeded on first run. Edit these, then delete the data dir to re-seed.
fn demo_profile() -> Profile {
    Profile {
        display_name: Some("Demo".into()),
        sex: Sex::Male,
        birth_date: NaiveDate::from_ymd_opt(1995, 6, 15).expect("valid date"),
        height_cm: 175.0,
        activity_level: ActivityLevel::Light,
        dietary_pattern: DietaryPattern::Omnivore,
        allergies: vec![],
        dislikes: vec![],
        medical_notes: None,
        meals_per_day: 3,
    }
}
const DEMO_WEIGHT_KG: f64 = 78.0;
fn demo_goal() -> WeightGoal {
    WeightGoal {
        direction: GoalDirection::Lose,
        target_weight_kg: Some(72.0),
        rate_kg_per_week: 0.5,
    }
}

const HELP: &str = "\
Type anything to chat, e.g.:
  早餐吃了两个鸡蛋和一杯牛奶        (logs a meal)
  我今天 77.5 公斤                  (logs weight)
  我不吃香菜，记住                  (memory note)
  帮我生成明天的计划 / 明天中午聚餐，帮我调整计划
Commands:
  /today            today's intake vs targets, meals, plans, weight
  /json             the same, as the raw TodayView JSON the UI will get
  /plan [date]      generate a plan (default tomorrow)
  /show [date]      show the current plan (default tomorrow)
  /revise <text>    revise tomorrow's plan
  /accept           accept tomorrow's plan
  /tick             run one scheduler tick (backup; plan if past 21:00)
  /profile          profile, goal, latest weight
  /trace on|off     show tool calls after each reply (default on)
  /quit";

#[tokio::main]
async fn main() -> Result<()> {
    let api_key = std::env::var("ANTHROPIC_API_KEY").map_err(|_| "set ANTHROPIC_API_KEY first")?;
    let model = std::env::var("ANTHROPIC_MODEL").unwrap_or_else(|_| "claude-sonnet-5".into());
    let data_dir = std::env::temp_dir().join("lifeagent-try");
    println!(
        "data dir: {}  (delete it to start over)",
        data_dir.display()
    );

    // The router is built from settings once, at build time, so write the model config first.
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
    Settings::new(db::open(&data_dir).await?)
        .set_models(&config, Utc::now())
        .await?;

    let secrets = Arc::new(MemorySecretStore::new());
    secrets.set("anthropic", &api_key)?;
    let app = CoreAppBuilder::new(data_dir)
        .secrets(secrets)
        .plugin(Arc::new(DietPlugin))
        .build()
        .await?;
    seed(&app).await?;

    println!("model: {model}\n\n{HELP}\n");
    let mut scheduler = Scheduler::new(app.clone());
    let mut trace = true;
    let stdin = io::stdin();
    loop {
        print!("> ");
        io::stdout().flush()?;
        let mut line = String::new();
        if stdin.lock().read_line(&mut line)? == 0 {
            break;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (cmd, arg) = line.split_once(' ').unwrap_or((line, ""));
        let arg = arg.trim();
        let result = match cmd {
            "/quit" | "/exit" => break,
            "/help" => {
                println!("{HELP}");
                Ok(())
            }
            "/today" => today(&app).await,
            "/json" => today_json(&app).await,
            "/plan" => generate(&app, arg).await,
            "/show" => show(&app, arg).await,
            "/revise" => revise(&app, arg).await,
            "/accept" => accept(&app).await,
            "/tick" => tick(&app, &mut scheduler).await,
            "/profile" => profile(&app).await,
            "/trace" => {
                trace = arg != "off";
                println!("trace {}", if trace { "on" } else { "off" });
                Ok(())
            }
            _ if cmd.starts_with('/') => {
                println!("unknown command; /help");
                Ok(())
            }
            _ => chat(&app, line, trace).await,
        };
        if let Err(e) = result {
            println!("error: {e}");
        }
        println!();
    }
    Ok(())
}

async fn seed(app: &CoreApp) -> Result<()> {
    if app.profile().get().await?.is_some() {
        return Ok(());
    }
    let now = Utc::now();
    app.profile().set(&demo_profile()).await?;
    app.events()
        .append(
            "body.weight_logged",
            now,
            json!({"kg": DEMO_WEIGHT_KG}),
            EventSource::Ui,
        )
        .await?;
    app.goals().save_weight_goal(&demo_goal()).await?;
    app.settings().set_onboarding_complete(true, now).await?;
    println!("seeded demo profile: male, 175 cm, 78 kg, light activity, lose 0.5 kg/week to 72 kg");
    Ok(())
}

async fn chat(app: &CoreApp, text: &str, trace: bool) -> Result<()> {
    println!("(thinking…)");
    let turn = app.send_message(text).await?;
    if trace {
        print_trace(app).await?;
    }
    println!("{}", turn.reply);
    for activity in &turn.activities {
        println!("  · {activity}");
    }
    Ok(())
}

/// Tool calls and results of the turn that just finished, from the conversation store.
async fn print_trace(app: &CoreApp) -> Result<()> {
    let messages = app.conversation().recent(40).await?;
    let start = messages
        .iter()
        .rposition(|m| m.role == MessageRole::User)
        .map_or(0, |i| i + 1);
    for m in &messages[start..] {
        match m.role {
            MessageRole::Assistant => {
                let calls: Vec<Value> = m
                    .tool_calls_json
                    .as_deref()
                    .map(serde_json::from_str)
                    .transpose()?
                    .unwrap_or_default();
                for call in calls {
                    println!(
                        "  → {} {}",
                        call["name"].as_str().unwrap_or("?"),
                        call["arguments"]
                    );
                }
            }
            MessageRole::Tool => {
                let content = m.content.as_deref().unwrap_or("");
                let short: String = content.chars().take(400).collect();
                let more = if content.chars().count() > 400 {
                    "…"
                } else {
                    ""
                };
                println!(
                    "  ← {}: {short}{more}",
                    m.tool_name.as_deref().unwrap_or("?")
                );
            }
            MessageRole::User => {}
        }
    }
    Ok(())
}

fn num(v: &Value) -> String {
    v.as_f64().map_or_else(|| "-".into(), |n| n.to_string())
}

fn meal_label(v: &Value) -> &'static str {
    v.as_str()
        .and_then(MealType::parse)
        .map_or("?", |m| m.zh_label())
}

async fn today(app: &CoreApp) -> Result<()> {
    let view = app.today_view().await?;
    let weight = view.latest_weight.as_ref().map_or("无".into(), |w| {
        format!("{} kg（{}）", w.kg, w.logical_date)
    });
    println!("{} · 最新体重 {weight}", view.logical_date);

    if let Some(card) = view.cards.get("diet") {
        let (t, g) = (&card["total"], &card["target"]);
        println!(
            "摄入 {} / {} kcal · 蛋白质 {} / {} g · 碳水 {} / {} g · 脂肪 {} / {} g · 纤维 {} / {} g",
            num(&t["kcal"]),
            num(&g["kcal"]),
            num(&t["protein_g"]),
            num(&g["protein_g"]),
            num(&t["carbs_g"]),
            num(&g["carbs_g"]),
            num(&t["fat_g"]),
            num(&g["fat_g"]),
            num(&t["fiber_g"]),
            num(&g["fiber_g"]),
        );
        let meals = card["meals"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();
        if meals.is_empty() {
            println!("  （今天还没有记录）");
        }
        for meal in meals {
            println!(
                "  {} {} {} kcal  [entry {}]",
                meal["time"].as_str().unwrap_or(""),
                meal_label(&meal["meal_type"]),
                num(&meal["total"]["kcal"]),
                meal["entry_id"].as_str().unwrap_or("")
            );
            print_items(&meal["items"]);
        }
    }
    match &view.today_plan {
        Some(p) => println!("今日计划：第 {} 版 · {}", p.version, p.content.summary),
        None => println!("今日计划：无"),
    }
    match &view.tomorrow_plan {
        Some(p) => println!(
            "明日计划：第 {} 版 · {}  （/show 查看）",
            p.version, p.content.summary
        ),
        None => println!("明日计划：无  （/plan 生成，或 21:00 后 /tick）"),
    }
    Ok(())
}

async fn today_json(app: &CoreApp) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(&app.today_view().await?)?
    );
    Ok(())
}

fn print_items(items: &Value) {
    for item in items.as_array().map(Vec::as_slice).unwrap_or_default() {
        let estimated = if item["estimated"] == Value::Bool(true) {
            " （估）"
        } else {
            ""
        };
        println!(
            "      - {} {} g · {} kcal · 蛋白质 {} g{estimated}",
            item["name"].as_str().unwrap_or("?"),
            num(&item["grams"]),
            num(&item["kcal"]),
            num(&item["protein_g"])
        );
    }
}

fn print_plan(plan: &StoredPlan) {
    println!(
        "{} 的计划 · 第 {} 版 · {:?} · {:?}",
        plan.plan_date, plan.version, plan.status, plan.trigger
    );
    if let Some(instruction) = &plan.instruction {
        println!("调整要求：{instruction}");
    }
    println!("{}", plan.content.summary);
    if let Some(diet) = plan.content.sections.get("diet") {
        for meal in diet["meals"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            println!(
                "  {} {} · {} kcal",
                meal_label(&meal["meal_type"]),
                meal["title"].as_str().unwrap_or(""),
                num(&meal["total"]["kcal"])
            );
            if let Some(note) = meal["prep_note"].as_str() {
                println!("      做法：{note}");
            }
            print_items(&meal["items"]);
        }
        let (t, g) = (&diet["total"], &diet["targets"]);
        println!(
            "  合计 {} / 目标 {} kcal · 蛋白质 {} / {} g · 碳水 {} / {} g · 脂肪 {} / {} g",
            num(&t["kcal"]),
            num(&g["kcal"]),
            num(&t["protein_g"]),
            num(&g["protein_g"]),
            num(&t["carbs_g"]),
            num(&g["carbs_g"]),
            num(&t["fat_g"]),
            num(&g["fat_g"]),
        );
        for tip in diet["tips"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            println!("  提示：{}", tip.as_str().unwrap_or(""));
        }
    }
    for warning in &plan.warnings {
        println!("  ! {warning}");
    }
    if let Some(d) = &plan.content.disclaimer {
        println!("  {d}");
    }
}

async fn date_or_tomorrow(app: &CoreApp, arg: &str) -> Result<NaiveDate> {
    if !arg.is_empty() {
        return Ok(NaiveDate::parse_from_str(arg, "%Y-%m-%d")?);
    }
    app.logical_today()
        .await?
        .succ_opt()
        .ok_or_else(|| "date out of range".into())
}

async fn generate(app: &CoreApp, arg: &str) -> Result<()> {
    let date = date_or_tomorrow(app, arg).await?;
    println!("(generating the plan for {date}; the planner can take a minute…)");
    print_plan(&app.generate_plan(date, PlanTrigger::Manual).await?);
    Ok(())
}

async fn show(app: &CoreApp, arg: &str) -> Result<()> {
    let date = date_or_tomorrow(app, arg).await?;
    match app.get_plan(date).await? {
        Some(plan) => print_plan(&plan),
        None => println!("no plan for {date}"),
    }
    Ok(())
}

async fn revise(app: &CoreApp, instruction: &str) -> Result<()> {
    let date = date_or_tomorrow(app, "").await?;
    println!("(revising {date}…)");
    print_plan(&app.revise_plan(date, instruction).await?);
    Ok(())
}

async fn accept(app: &CoreApp) -> Result<()> {
    let date = date_or_tomorrow(app, "").await?;
    app.accept_plan(date).await?;
    println!("accepted the plan for {date}");
    Ok(())
}

async fn tick(app: &CoreApp, scheduler: &mut Scheduler) -> Result<()> {
    let tomorrow = date_or_tomorrow(app, "").await?;
    let had_plan = app.get_plan(tomorrow).await?.is_some();
    println!("(tick…)");
    scheduler.tick().await;
    let today = app.logical_today().await?;
    let has_plan = app.get_plan(tomorrow).await?.is_some();
    println!(
        "backup for {today}: taken or already present · plan for {tomorrow}: {}",
        match (had_plan, has_plan) {
            (false, true) => "generated now (trigger schedule)",
            (true, _) => "already existed",
            (false, false) => "not generated (before plan_time 21:00, or it failed)",
        }
    );
    Ok(())
}

async fn profile(app: &CoreApp) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "profile": app.profile().get().await?,
            "goal": app.goals().get_active_weight_goal().await?,
            "latest_weight": app.events().latest("body.weight_logged").await?.map(|e| e.payload),
        }))?
    );
    Ok(())
}
