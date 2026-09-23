//! Conversation store and history window, per `docs/PHASE1_SPEC.md` §7.10.
//!
//! Deliberately independent of `lifeagent-llm::Message`: the `messages` table is already a
//! flat, row-shaped schema (role/content/tool_calls_json/tool_call_id/tool_name), so this
//! module speaks in terms of that shape. Translating a `StoredMessage` into a
//! `lifeagent-llm::Message` for a `ChatRequest` is the orchestrator's job (M3), which is also
//! where `lifeagent-llm`'s types first become available to this crate.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::clock::Clock;
use crate::error::CoreError;

/// How many messages `history_window` returns at most (§7.10).
const HISTORY_WINDOW: u32 = 30;

/// Tool message `content` is truncated to this many characters at storage time (§7.10).
const TOOL_CONTENT_MAX_CHARS: usize = 4000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    #[default]
    User,
    Assistant,
    Tool,
}

impl MessageRole {
    fn as_str(&self) -> &'static str {
        match self {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::Tool => "tool",
        }
    }

    fn parse(s: &str) -> Result<Self, CoreError> {
        match s {
            "user" => Ok(MessageRole::User),
            "assistant" => Ok(MessageRole::Assistant),
            "tool" => Ok(MessageRole::Tool),
            other => Err(CoreError::Validation(format!(
                "unknown message role: {other:?}"
            ))),
        }
    }
}

/// A message as given to [`ConversationStore::append`]. `id` and `created_at` are assigned
/// by the store.
#[derive(Debug, Clone, Default)]
pub struct NewMessage {
    pub role: MessageRole,
    pub content: Option<String>,
    pub tool_calls_json: Option<String>,
    pub tool_call_id: Option<String>,
    pub tool_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StoredMessage {
    pub id: String,
    pub role: MessageRole,
    pub content: Option<String>,
    pub tool_calls_json: Option<String>,
    pub tool_call_id: Option<String>,
    pub tool_name: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone)]
pub struct ConversationStore {
    db: SqlitePool,
    clock: Arc<dyn Clock>,
}

/// Raw column tuple for one `messages` row: (id, role, content, tool_calls_json,
/// tool_call_id, tool_name, created_at).
type MessageRow = (
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    String,
);

impl ConversationStore {
    pub fn new(db: SqlitePool, clock: Arc<dyn Clock>) -> Self {
        Self { db, clock }
    }

    pub async fn append(&self, mut msg: NewMessage) -> Result<StoredMessage, CoreError> {
        if msg.role == MessageRole::Tool {
            if let Some(content) = &msg.content {
                if content.chars().count() > TOOL_CONTENT_MAX_CHARS {
                    msg.content = Some(content.chars().take(TOOL_CONTENT_MAX_CHARS).collect());
                }
            }
        }

        let id = Uuid::now_v7().to_string();
        let created_at = self.clock.now();
        sqlx::query(
            "INSERT INTO messages (id, role, content, tool_calls_json, tool_call_id, tool_name, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(msg.role.as_str())
        .bind(&msg.content)
        .bind(&msg.tool_calls_json)
        .bind(&msg.tool_call_id)
        .bind(&msg.tool_name)
        .bind(created_at.to_rfc3339())
        .execute(&self.db)
        .await?;

        Ok(StoredMessage {
            id,
            role: msg.role,
            content: msg.content,
            tool_calls_json: msg.tool_calls_json,
            tool_call_id: msg.tool_call_id,
            tool_name: msg.tool_name,
            created_at,
        })
    }

    /// The most recent `limit` messages, oldest first.
    pub async fn recent(&self, limit: u32) -> Result<Vec<StoredMessage>, CoreError> {
        // SQLite has no offset-from-end syntax, so take the last N by ROWID (SQLite's
        // implicit insertion-order key) in a subquery, then re-sort ascending.
        let rows: Vec<MessageRow> = sqlx::query_as(
                "SELECT id, role, content, tool_calls_json, tool_call_id, tool_name, created_at FROM (
                     SELECT rowid, id, role, content, tool_calls_json, tool_call_id, tool_name, created_at
                     FROM messages ORDER BY rowid DESC LIMIT ?
                 ) ORDER BY rowid ASC",
            )
            .bind(limit)
            .fetch_all(&self.db)
            .await?;

        rows.into_iter().map(Self::row_to_message).collect()
    }

    /// The history window sent to the conversation model (§7.10): the last 30 stored
    /// messages, trimmed so the window starts at a `user` message — never with an orphan
    /// `tool` message at the front.
    pub async fn history_window(&self) -> Result<Vec<StoredMessage>, CoreError> {
        let messages = self.recent(HISTORY_WINDOW).await?;
        let start = messages.iter().position(|m| m.role == MessageRole::User);
        match start {
            Some(i) => Ok(messages[i..].to_vec()),
            None => Ok(Vec::new()),
        }
    }

    fn row_to_message(row: MessageRow) -> Result<StoredMessage, CoreError> {
        let (id, role, content, tool_calls_json, tool_call_id, tool_name, created_at) = row;
        Ok(StoredMessage {
            id,
            role: MessageRole::parse(&role)?,
            content,
            tool_calls_json,
            tool_call_id,
            tool_name,
            created_at: DateTime::parse_from_rfc3339(&created_at)
                .map_err(|e| CoreError::Validation(format!("bad created_at in db: {e}")))?
                .with_timezone(&Utc),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(content: &str) -> NewMessage {
        NewMessage {
            role: MessageRole::User,
            content: Some(content.to_string()),
            ..Default::default()
        }
    }

    fn assistant(content: &str) -> NewMessage {
        NewMessage {
            role: MessageRole::Assistant,
            content: Some(content.to_string()),
            ..Default::default()
        }
    }

    fn tool(content: &str, tool_call_id: &str) -> NewMessage {
        NewMessage {
            role: MessageRole::Tool,
            content: Some(content.to_string()),
            tool_call_id: Some(tool_call_id.to_string()),
            ..Default::default()
        }
    }

    async fn store() -> (ConversationStore, tempfile::TempDir) {
        let (pool, dir) = crate::db::test_pool().await;
        (
            ConversationStore::new(pool, Arc::new(crate::clock::FixedClock::new(Utc::now()))),
            dir,
        )
    }

    #[tokio::test]
    async fn append_then_recent_round_trips() {
        let (store, _dir) = store().await;
        let appended = store.append(user("早上吃了燕麦")).await.unwrap();

        let recent = store.recent(10).await.unwrap();
        assert_eq!(recent, vec![appended]);
    }

    #[tokio::test]
    async fn recent_returns_oldest_first_within_the_limit() {
        let (store, _dir) = store().await;
        store.append(user("一")).await.unwrap();
        store.append(assistant("二")).await.unwrap();
        let third = store.append(user("三")).await.unwrap();

        let recent = store.recent(2).await.unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[1], third);
        assert_eq!(recent[0].content.as_deref(), Some("二"));
    }

    #[tokio::test]
    async fn tool_message_content_is_truncated_at_storage() {
        let (store, _dir) = store().await;
        let long = "a".repeat(TOOL_CONTENT_MAX_CHARS + 500);

        let appended = store.append(tool(&long, "call_1")).await.unwrap();
        assert_eq!(
            appended.content.unwrap().chars().count(),
            TOOL_CONTENT_MAX_CHARS
        );
    }

    #[tokio::test]
    async fn non_tool_message_content_is_not_truncated() {
        let (store, _dir) = store().await;
        let long = "a".repeat(TOOL_CONTENT_MAX_CHARS + 500);

        let appended = store.append(assistant(&long)).await.unwrap();
        assert_eq!(
            appended.content.unwrap().chars().count(),
            TOOL_CONTENT_MAX_CHARS + 500
        );
    }

    #[tokio::test]
    async fn history_window_starts_at_the_first_user_message() {
        let (store, _dir) = store().await;
        // Simulate a window that would otherwise begin with an orphan tool message: a
        // leading assistant/tool pair from a turn whose anchoring user message fell outside
        // the last 30 rows.
        store
            .append(assistant("leftover from a previous turn"))
            .await
            .unwrap();
        store
            .append(tool("leftover result", "call_0"))
            .await
            .unwrap();
        let anchor = store.append(user("这次的问题")).await.unwrap();
        let reply = store.append(assistant("这次的回答")).await.unwrap();

        let window = store.history_window().await.unwrap();
        assert_eq!(window, vec![anchor, reply]);
    }

    #[tokio::test]
    async fn history_window_is_empty_when_no_user_message_is_present() {
        let (store, _dir) = store().await;
        store.append(assistant("只有助手消息")).await.unwrap();

        assert_eq!(store.history_window().await.unwrap(), vec![]);
    }

    #[tokio::test]
    async fn history_window_caps_at_30_messages() {
        let (store, _dir) = store().await;
        // 35 user messages in a row: every one qualifies as a valid window start, so this
        // isolates the cap itself from the trim-to-user-start rule (covered separately above).
        for i in 0..35 {
            store.append(user(&format!("第 {i} 条"))).await.unwrap();
        }

        let window = store.history_window().await.unwrap();
        assert_eq!(window.len(), HISTORY_WINDOW as usize);
        assert!(window.iter().all(|m| m.role == MessageRole::User));
        // The oldest 5 of the 35 fell outside the window.
        assert_eq!(window[0].content.as_deref(), Some("第 5 条"));
        assert_eq!(window[29].content.as_deref(), Some("第 34 条"));
    }
}
