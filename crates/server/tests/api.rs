//! The web server end to end: a real server on a random local port, a real Postgres, the echo
//! model. Each test gets its own `test_auth_<uuid>` auth schema; the users it registers get
//! their own tenants, all removed when the test's `TestServer` drops.

use std::sync::Arc;
use std::time::Duration;

use lifeagent_core::clock::SystemClock;
use lifeagent_core::db::{destroy, test_login_url, DbConfig};
use lifeagent_server::config::ServerConfig;
use lifeagent_server::{router, AppState};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::{AssertSqlSafe, Connection, PgConnection};

const TICK_TOKEN: &str = "tick-secret";

struct TestServer {
    base: String,
    http: reqwest::Client,
    url: String,
    auth_schema: String,
    state: Option<Arc<AppState>>,
}

async fn start_with(edit: impl FnOnce(&mut ServerConfig)) -> TestServer {
    let url = test_login_url().await;
    let auth_schema = format!("test_auth_{}", uuid::Uuid::now_v7().simple());
    let mut config = ServerConfig {
        database_url: url.clone(),
        auth_schema: auth_schema.clone(),
        bind: "127.0.0.1:0".parse().unwrap(),
        static_dir: None,
        models: None,
        mock_llm: true,
        tick_token: Some(TICK_TOKEN.into()),
        inprocess_scheduler: false,
        secure_cookies: false,
    };
    edit(&mut config);
    let state = AppState::new(config, Arc::new(SystemClock)).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = router(state.clone());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    TestServer {
        base,
        http: reqwest::Client::new(),
        url,
        auth_schema,
        state: Some(state),
    }
}

async fn start() -> TestServer {
    start_with(|_| {}).await
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.state.take();
        let url = self.url.clone();
        let auth_schema = self.auth_schema.clone();
        let _ = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async {
                let mut conn = PgConnection::connect(&url).await.unwrap();
                let schemas: Vec<String> = sqlx::query_scalar(AssertSqlSafe(format!(
                    "SELECT schema_name FROM \"{auth_schema}\".users"
                )))
                .fetch_all(&mut conn)
                .await
                .unwrap_or_default();
                for schema in schemas {
                    let _ = destroy(&DbConfig::tenant(url.clone(), schema)).await;
                }
                let _ = sqlx::query(AssertSqlSafe(format!(
                    "DROP SCHEMA IF EXISTS \"{auth_schema}\" CASCADE"
                )))
                .execute(&mut conn)
                .await;
            });
        })
        .join();
    }
}

/// A logged-in browser: just the session cookie.
#[derive(Clone)]
struct Session {
    cookie: Option<String>,
}

fn session_cookie(resp: &reqwest::Response) -> Option<String> {
    resp.headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find_map(|v| v.split(';').next())
        .map(str::to_string)
}

impl TestServer {
    async fn post(&self, session: &Session, path: &str, body: Value) -> (StatusCode, Value) {
        let mut req = self.http.post(format!("{}{path}", self.base)).json(&body);
        if let Some(cookie) = &session.cookie {
            req = req.header("cookie", cookie);
        }
        let resp = req.send().await.unwrap();
        let status = resp.status();
        let body = resp.json().await.unwrap_or(Value::Null);
        (status, body)
    }

    async fn auth(&self, path: &str, username: &str, password: &str) -> (StatusCode, Session) {
        let resp = self
            .http
            .post(format!("{}/api/auth/{path}", self.base))
            .json(&json!({ "username": username, "password": password }))
            .send()
            .await
            .unwrap();
        let status = resp.status();
        (
            status,
            Session {
                cookie: session_cookie(&resp),
            },
        )
    }

    async fn register(&self, username: &str) -> Session {
        let (status, session) = self.auth("register", username, "password123").await;
        assert_eq!(status, StatusCode::OK);
        session
    }

    async fn me(&self, session: &Session) -> StatusCode {
        let mut req = self.http.get(format!("{}/api/auth/me", self.base));
        if let Some(cookie) = &session.cookie {
            req = req.header("cookie", cookie);
        }
        req.send().await.unwrap().status()
    }

    async fn admin(&self) -> PgConnection {
        PgConnection::connect(&self.url).await.unwrap()
    }

    async fn schema_of(&self, username: &str) -> String {
        sqlx::query_scalar(AssertSqlSafe(format!(
            "SELECT schema_name FROM \"{}\".users WHERE username = $1",
            self.auth_schema
        )))
        .bind(username)
        .fetch_one(&mut self.admin().await)
        .await
        .unwrap()
    }
}

#[tokio::test]
async fn register_me_logout_login_round_trip() {
    let server = start().await;
    let session = server.register("Alice").await;
    assert!(session
        .cookie
        .as_deref()
        .unwrap()
        .starts_with("lifeagent_session="));
    assert_eq!(server.me(&session).await, StatusCode::OK);

    let (status, _) = server.post(&session, "/api/auth/logout", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(server.me(&session).await, StatusCode::UNAUTHORIZED);

    let (status, _) = server.auth("login", "alice", "wrong-password").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = server.auth("login", "nobody", "password123").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Usernames are case-insensitive.
    let (status, session) = server.auth("login", "ALICE", "password123").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(server.me(&session).await, StatusCode::OK);
}

#[tokio::test]
async fn usernames_are_unique_and_inputs_are_validated() {
    let server = start().await;
    server.register("bob").await;
    let none = Session { cookie: None };

    let (status, body) = server
        .post(
            &none,
            "/api/auth/register",
            json!({ "username": "BOB", "password": "password123" }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "username_taken");
    assert_eq!(body["detail"]["kind"], "username_taken");

    let (status, body) = server
        .post(
            &none,
            "/api/auth/register",
            json!({ "username": "carol", "password": "short" }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "invalid_input");
    assert_eq!(body["detail"]["kind"], "invalid_password");
}

#[tokio::test]
async fn health_answers_without_a_session() {
    let server = start().await;
    let resp = server
        .http
        .get(format!("{}/health", server.base))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.text().await.unwrap(), "ok");
}

#[tokio::test]
async fn commands_need_a_session() {
    let server = start().await;
    let none = Session { cookie: None };
    let (status, body) = server.post(&none, "/api/get_today", json!({})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["code"], "unauthorized");

    let forged = Session {
        cookie: Some("lifeagent_session=00ff".into()),
    };
    let (status, _) = server.post(&forged, "/api/get_today", json!({})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn each_user_sees_only_their_own_data() {
    let server = start().await;
    let alice = server.register("alice").await;
    let bob = server.register("bob").await;

    let (status, _) = server
        .post(&alice, "/api/log_weight", json!({ "kg": 70.5 }))
        .await;
    assert_eq!(status, StatusCode::OK);

    let (_, today) = server.post(&alice, "/api/get_today", json!({})).await;
    assert_eq!(today["latest_weight"]["kg"], 70.5);
    let (_, today) = server.post(&bob, "/api/get_today", json!({})).await;
    assert_eq!(today["latest_weight"], Value::Null);

    // Each user's tables are owned by their own role, in their own schema.
    let schema = server.schema_of("alice").await;
    assert!(schema.starts_with("u_"));
    let owners: Vec<String> =
        sqlx::query_scalar("SELECT DISTINCT tableowner::TEXT FROM pg_tables WHERE schemaname = $1")
            .bind(&schema)
            .fetch_all(&mut server.admin().await)
            .await
            .unwrap();
    assert_eq!(owners, vec![schema]);
}

#[tokio::test]
async fn chat_round_trips_through_the_echo_model() {
    let server = start().await;
    let alice = server.register("alice").await;

    let (status, turn) = server
        .post(&alice, "/api/send_message", json!({ "text": "早上好" }))
        .await;
    assert_eq!(status, StatusCode::OK);
    // The model sees the message tagged with its send time; the echo repeats that.
    let reply = turn["reply"].as_str().unwrap();
    assert!(
        reply.starts_with("(mock echo) [sent ") && reply.ends_with("] 早上好"),
        "{reply}"
    );

    let (_, history) = server
        .post(&alice, "/api/get_history", json!({ "limit": 10 }))
        .await;
    let roles: Vec<&str> = history
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["role"].as_str().unwrap())
        .collect();
    assert_eq!(roles, vec!["user", "assistant"]);

    let (_, state) = server.post(&alice, "/api/get_app_state", json!({})).await;
    assert_eq!(state["models_configured"], true);
    assert_eq!(state["onboarding_complete"], false);
}

#[tokio::test]
async fn bad_arguments_unknown_and_operator_only_commands_are_rejected() {
    let server = start().await;
    let alice = server.register("alice").await;

    let (status, body) = server
        .post(&alice, "/api/log_weight", json!({ "kilos": 70 }))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "invalid_input");

    let (status, body) = server
        .post(&alice, "/api/log_weight", json!({ "kg": 5 }))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    for command in [
        "save_models",
        "set_api_key",
        "get_models",
        "no_such_command",
    ] {
        let (status, body) = server
            .post(&alice, &format!("/api/{command}"), json!({}))
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{command}");
        assert_eq!(body["code"], "not_found");
    }
}

#[tokio::test]
async fn deleting_an_account_removes_its_data_role_and_sessions() {
    let server = start().await;
    let alice = server.register("alice").await;
    let schema = server.schema_of("alice").await;

    let (status, body) = server
        .post(
            &alice,
            "/api/auth/delete_account",
            json!({ "password": "wrong-password" }),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["code"], "invalid_credentials");

    let (status, _) = server
        .post(
            &alice,
            "/api/auth/delete_account",
            json!({ "password": "password123" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(server.me(&alice).await, StatusCode::UNAUTHORIZED);
    let (status, _) = server.auth("login", "alice", "password123").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (schemas, roles): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM pg_namespace WHERE nspname = $1),
                (SELECT COUNT(*) FROM pg_roles WHERE rolname = $1)",
    )
    .bind(&schema)
    .fetch_one(&mut server.admin().await)
    .await
    .unwrap();
    assert_eq!((schemas, roles), (0, 0));

    // The name is free again.
    server.register("alice").await;
}

#[tokio::test]
async fn the_tick_endpoint_needs_its_token_and_visits_every_user() {
    let server = start().await;
    server.register("alice").await;
    server.register("bob").await;

    let tick = |token: Option<&str>| {
        let mut req = server
            .http
            .post(format!("{}/api/internal/tick", server.base));
        if let Some(token) = token {
            req = req.header("authorization", format!("Bearer {token}"));
        }
        req.send()
    };
    assert_eq!(tick(None).await.unwrap().status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        tick(Some("wrong")).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    let resp = tick(Some(TICK_TOKEN)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let report: Value = resp.json().await.unwrap();
    assert_eq!(report, json!({ "users": 2, "failed": 0 }));

    // The custom header is what Cloud Scheduler sends.
    let resp = server
        .http
        .post(format!("{}/api/internal/tick", server.base))
        .header("x-lifeagent-tick-token", TICK_TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn the_tick_endpoint_is_off_without_a_token() {
    let server = start_with(|c| c.tick_token = None).await;
    let resp = server
        .http
        .post(format!("{}/api/internal/tick", server.base))
        .header("authorization", "Bearer anything")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn set_language_accepts_only_the_ui_locales() {
    let server = start().await;
    let alice = server.register("alice").await;

    for language in ["ja", "en", "zh-CN"] {
        let (status, body) = server
            .post(&alice, "/api/set_language", json!({ "language": language }))
            .await;
        assert_eq!(status, StatusCode::OK, "{language}: {body}");
    }
    let (status, body) = server
        .post(&alice, "/api/set_language", json!({ "language": "fr" }))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "invalid_input");
}

#[tokio::test]
async fn the_event_stream_reports_data_changes() {
    let server = start().await;
    let alice = server.register("alice").await;

    let mut stream = server
        .http
        .get(format!("{}/api/events", server.base))
        .header("cookie", alice.cookie.clone().unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(stream.status(), StatusCode::OK);
    assert_eq!(
        stream.headers()["content-type"].to_str().unwrap(),
        "text/event-stream"
    );

    let (status, _) = server
        .post(&alice, "/api/log_weight", json!({ "kg": 70.0 }))
        .await;
    assert_eq!(status, StatusCode::OK);

    let mut received = String::new();
    let found = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(chunk) = stream.chunk().await.unwrap() {
            received.push_str(&String::from_utf8_lossy(&chunk));
            if received.contains("event: data-changed") {
                return true;
            }
        }
        false
    })
    .await;
    assert_eq!(found, Ok(true), "{received}");
    assert!(received.contains("body.weight_logged"), "{received}");
}

#[tokio::test]
async fn food_import_previews_and_commits_over_http() {
    let server = start().await;
    let alice = server.register("alice").await;

    let csv = "name,kcal,protein_g,carbs_g,fat_g,fiber_g\n芹菜,16,0.8,3.3,0.2,1.4\n";
    let resp = server
        .http
        .post(format!("{}/api/preview_food_import", server.base))
        .header("cookie", alice.cookie.clone().unwrap())
        .header("x-file-name", "%E8%94%AC%E8%8F%9C.csv")
        .body(csv)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let preview: Value = resp.json().await.unwrap();
    let token = preview["token"].as_str().unwrap().to_string();

    let (status, report) = server
        .post(
            &alice,
            "/api/commit_food_import",
            json!({ "token": token, "name": "蔬菜" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{report}");

    let (_, sources) = server
        .post(&alice, "/api/list_food_sources", json!({}))
        .await;
    assert!(
        sources
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["name"] == "蔬菜"),
        "{sources}"
    );

    // A token is single-use.
    let (status, _) = server
        .post(&alice, "/api/commit_food_import", json!({ "token": token }))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn stock_items_can_be_added_changed_and_removed() {
    let server = start().await;
    let session = server.register("pantry").await;

    let (status, item) = server
        .post(
            &session,
            "/api/add_stock_item",
            json!({ "item": { "name": "鸡蛋", "quantity": 10, "unit": "piece", "expires_on": "2026-10-20" } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(item["quantity"], 10.0);
    let id = item["id"].as_str().unwrap().to_string();

    let (status, _) = server
        .post(
            &session,
            "/api/set_stock_quantity",
            json!({ "id": id, "quantity": 4 }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, view) = server.post(&session, "/api/get_stock", json!({})).await;
    assert_eq!(view["items"][0]["quantity"], 4.0);
    assert_eq!(view["settings"]["auto_consume"], true);
    assert!(view["shopping"].is_null());

    let (status, body) = server
        .post(
            &session,
            "/api/add_stock_item",
            json!({ "item": { "name": "牛奶", "quantity": 0, "unit": "ml" } }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["detail"]["kind"], "invalid_input");

    let (status, _) = server
        .post(
            &session,
            "/api/save_stock_settings",
            json!({ "settings": { "auto_consume": false, "plan_from_stock": true } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = server
        .post(&session, "/api/delete_stock_item", json!({ "id": id }))
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, view) = server.post(&session, "/api/get_stock", json!({})).await;
    assert_eq!(view["items"], json!([]));
    assert_eq!(view["settings"]["auto_consume"], false);
}
