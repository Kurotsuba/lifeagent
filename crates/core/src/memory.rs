//! Memory notes: durable preferences and facts the agent should remember, per
//! `docs/PHASE1_SPEC.md` §7 (`PluginContext.memory`) and the `core_remember` tool
//! description in §7.8 ("Do not use for one-off events" — enforced by callers, not here).

use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::clock::Clock;
use crate::error::{CoreError, ValidationRule};

/// Matches the `core_remember` tool's stated limit (§7.8).
pub const MAX_NOTE_LEN: usize = 300;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MemoryNote {
    pub id: String,
    pub note: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone)]
pub struct MemoryService {
    db: sqlx::SqlitePool,
    clock: Arc<dyn Clock>,
}

impl MemoryService {
    pub fn new(db: sqlx::SqlitePool, clock: Arc<dyn Clock>) -> Self {
        Self { db, clock }
    }

    /// Adds a note. Rejects empty (after trimming) or over-length text.
    pub async fn add(&self, note: &str) -> Result<MemoryNote, CoreError> {
        let trimmed = note.trim();
        if trimmed.is_empty() {
            return Err(CoreError::InvalidInput(ValidationRule::NoteEmpty));
        }
        if trimmed.chars().count() > MAX_NOTE_LEN {
            return Err(CoreError::InvalidInput(ValidationRule::NoteTooLong {
                got: trimmed.chars().count(),
                max: MAX_NOTE_LEN,
            }));
        }

        let id = Uuid::now_v7().to_string();
        let created_at = self.clock.now();
        sqlx::query(
            "INSERT INTO memory_notes (id, note, created_at, deleted_at) VALUES (?, ?, ?, NULL)",
        )
        .bind(&id)
        .bind(trimmed)
        .bind(created_at.to_rfc3339())
        .execute(&self.db)
        .await?;

        Ok(MemoryNote {
            id,
            note: trimmed.to_string(),
            created_at,
        })
    }

    /// Newest-first, excluding soft-deleted notes. §7.9 caps this at 50 for the context
    /// builder (M3); callers pick the limit that fits their use.
    pub async fn list(&self, limit: u32) -> Result<Vec<MemoryNote>, CoreError> {
        // `rowid DESC` breaks ties when two notes share the same `created_at` (e.g. under a
        // `FixedClock` that hasn't advanced between calls) — otherwise SQLite's tie order is
        // unspecified and "newest first" would not hold.
        let rows: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT id, note, created_at FROM memory_notes
             WHERE deleted_at IS NULL ORDER BY created_at DESC, rowid DESC LIMIT ?",
        )
        .bind(limit)
        .fetch_all(&self.db)
        .await?;

        rows.into_iter()
            .map(|(id, note, created_at)| {
                Ok(MemoryNote {
                    id,
                    note,
                    created_at: DateTime::parse_from_rfc3339(&created_at)
                        .map_err(|e| CoreError::Validation(format!("bad created_at in db: {e}")))?
                        .with_timezone(&Utc),
                })
            })
            .collect()
    }

    /// Soft-deletes a note (`core_forget`). Errors `NotFound` if it does not exist or was
    /// already forgotten.
    pub async fn forget(&self, note_id: &str) -> Result<(), CoreError> {
        let now = self.clock.now().to_rfc3339();
        let result = sqlx::query(
            "UPDATE memory_notes SET deleted_at = ? WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(now)
        .bind(note_id)
        .execute(&self.db)
        .await?;

        if result.rows_affected() == 0 {
            return Err(CoreError::NotFound(format!("memory note {note_id}")));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn service() -> (MemoryService, tempfile::TempDir) {
        let (pool, dir) = crate::db::test_pool().await;
        (
            MemoryService::new(pool, Arc::new(crate::clock::FixedClock::new(Utc::now()))),
            dir,
        )
    }

    #[tokio::test]
    async fn add_then_list_round_trips() {
        let (service, _dir) = service().await;
        let added = service.add("不吃香菜").await.unwrap();

        let notes = service.list(50).await.unwrap();
        assert_eq!(notes, vec![added]);
    }

    #[tokio::test]
    async fn add_rejects_empty_note() {
        let (service, _dir) = service().await;
        assert!(service.add("").await.is_err());
        assert!(service.add("   ").await.is_err());
    }

    #[tokio::test]
    async fn add_rejects_note_over_max_length() {
        let (service, _dir) = service().await;
        let too_long = "a".repeat(MAX_NOTE_LEN + 1);
        assert!(service.add(&too_long).await.is_err());
    }

    #[tokio::test]
    async fn add_accepts_note_at_max_length() {
        let (service, _dir) = service().await;
        let exactly = "a".repeat(MAX_NOTE_LEN);
        assert!(service.add(&exactly).await.is_ok());
    }

    #[tokio::test]
    async fn add_trims_whitespace() {
        let (service, _dir) = service().await;
        let added = service.add("  不吃香菜  ").await.unwrap();
        assert_eq!(added.note, "不吃香菜");
    }

    #[tokio::test]
    async fn list_is_newest_first() {
        let (service, _dir) = service().await;
        let first = service.add("第一条").await.unwrap();
        let second = service.add("第二条").await.unwrap();

        let notes = service.list(50).await.unwrap();
        assert_eq!(notes, vec![second, first]);
    }

    #[tokio::test]
    async fn list_respects_limit() {
        let (service, _dir) = service().await;
        service.add("一").await.unwrap();
        service.add("二").await.unwrap();
        service.add("三").await.unwrap();

        let notes = service.list(2).await.unwrap();
        assert_eq!(notes.len(), 2);
    }

    #[tokio::test]
    async fn forget_removes_note_from_list() {
        let (service, _dir) = service().await;
        let added = service.add("不吃香菜").await.unwrap();

        service.forget(&added.id).await.unwrap();
        assert_eq!(service.list(50).await.unwrap(), vec![]);
    }

    #[tokio::test]
    async fn forget_missing_note_is_not_found() {
        let (service, _dir) = service().await;
        assert!(matches!(
            service.forget("nope").await,
            Err(CoreError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn forget_twice_is_not_found_the_second_time() {
        let (service, _dir) = service().await;
        let added = service.add("不吃香菜").await.unwrap();
        service.forget(&added.id).await.unwrap();
        assert!(matches!(
            service.forget(&added.id).await,
            Err(CoreError::NotFound(_))
        ));
    }
}
