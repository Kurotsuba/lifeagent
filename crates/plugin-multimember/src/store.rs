//! The `multimember_*` tables (`docs/MULTIMEMBER_SPEC.md` §4): members, their dislikes, and
//! weekly default and one-off attendance.

use std::collections::BTreeMap;

use chrono::{DateTime, Datelike, NaiveDate, Utc};
use lifeagent_core::plugin::{PluginContext, PluginError, PluginMigration};
use sqlx::Row;
use uuid::Uuid;

use crate::model::{
    enum_str, parse_enum, AttendanceOverride, DefaultSlot, Dislike, MealType, Member, MemberError,
    MemberProfile, MemberRule, MAX_DISLIKE_CHARS, MAX_MEMBERS, MAX_NOTE_CHARS,
};

pub const MIGRATIONS: [PluginMigration; 4] = [
    PluginMigration {
        version: 1,
        sql: "CREATE TABLE multimember_members (
            id               TEXT PRIMARY KEY,
            seq              BIGINT GENERATED ALWAYS AS IDENTITY,
            display_name     TEXT NOT NULL,
            sex              TEXT NOT NULL,
            birth_date       TEXT NOT NULL,
            height_cm        DOUBLE PRECISION NOT NULL,
            weight_kg        DOUBLE PRECISION NOT NULL,
            activity_level   TEXT NOT NULL,
            dietary_pattern  TEXT NOT NULL,
            allergies_json   TEXT NOT NULL,
            created_at       TEXT NOT NULL,
            updated_at       TEXT NOT NULL,
            archived_at      TEXT
        )",
    },
    PluginMigration {
        version: 2,
        sql: "CREATE TABLE multimember_dislikes (
            id           TEXT PRIMARY KEY,
            seq          BIGINT GENERATED ALWAYS AS IDENTITY,
            member_id    TEXT NOT NULL REFERENCES multimember_members(id),
            text         TEXT NOT NULL,
            valid_from   TEXT,
            valid_until  TEXT,
            created_at   TEXT NOT NULL,
            deleted_at   TEXT
        )",
    },
    PluginMigration {
        version: 3,
        sql: "CREATE TABLE multimember_attendance_defaults (
            member_id  TEXT NOT NULL REFERENCES multimember_members(id),
            weekday    BIGINT NOT NULL,
            meal_type  TEXT NOT NULL,
            present    BIGINT NOT NULL,
            PRIMARY KEY (member_id, weekday, meal_type)
        )",
    },
    PluginMigration {
        version: 4,
        sql: "CREATE TABLE multimember_attendance_overrides (
            member_id  TEXT NOT NULL REFERENCES multimember_members(id),
            plan_date  TEXT NOT NULL,
            meal_type  TEXT NOT NULL,
            present    BIGINT NOT NULL,
            note       TEXT,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (member_id, plan_date, meal_type)
        )",
    },
];

/// `SELECT <every member column> FROM multimember_members <rest>`, as a `&'static str` (sqlx
/// only takes static SQL).
macro_rules! select_members {
    ($rest:literal) => {
        concat!(
            "SELECT id, display_name, sex, birth_date, height_cm, weight_kg, activity_level, ",
            "dietary_pattern, allergies_json, created_at, updated_at FROM multimember_members ",
            $rest
        )
    };
}

fn parse_date(s: &str) -> Result<NaiveDate, PluginError> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|e| PluginError::Other(format!("bad stored date {s:?}: {e}")))
}

fn parse_instant(s: &str) -> Result<DateTime<Utc>, PluginError> {
    DateTime::parse_from_rfc3339(s)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|e| PluginError::Other(format!("bad stored instant {s:?}: {e}")))
}

fn parse_meal(s: &str) -> Result<MealType, PluginError> {
    MealType::parse(s).ok_or_else(|| PluginError::Other(format!("bad stored meal_type {s:?}")))
}

fn member_from_row(row: &sqlx::postgres::PgRow) -> Result<Member, PluginError> {
    let allergies_json: String = row.try_get("allergies_json")?;
    Ok(Member {
        id: row.try_get("id")?,
        profile: MemberProfile {
            display_name: row.try_get("display_name")?,
            sex: parse_enum(&row.try_get::<String, _>("sex")?)?,
            birth_date: parse_date(&row.try_get::<String, _>("birth_date")?)?,
            height_cm: row.try_get("height_cm")?,
            weight_kg: row.try_get("weight_kg")?,
            activity_level: parse_enum(&row.try_get::<String, _>("activity_level")?)?,
            dietary_pattern: parse_enum(&row.try_get::<String, _>("dietary_pattern")?)?,
            allergies: serde_json::from_str(&allergies_json)
                .map_err(|e| PluginError::Other(format!("bad stored allergies: {e}")))?,
        },
        created_at: parse_instant(&row.try_get::<String, _>("created_at")?)?,
        updated_at: parse_instant(&row.try_get::<String, _>("updated_at")?)?,
    })
}

/// Non-archived members, in the order they were added.
pub async fn list_members(ctx: &PluginContext) -> Result<Vec<Member>, PluginError> {
    let rows = sqlx::query(select_members!("WHERE archived_at IS NULL ORDER BY seq"))
        .fetch_all(&ctx.db)
        .await?;
    rows.iter().map(member_from_row).collect()
}

/// A non-archived member by id.
pub async fn get_member(ctx: &PluginContext, id: &str) -> Result<Member, MemberError> {
    let row = sqlx::query(select_members!("WHERE id = $1 AND archived_at IS NULL"))
        .bind(id)
        .fetch_optional(&ctx.db)
        .await?;
    match row {
        Some(row) => Ok(member_from_row(&row)?),
        None => Err(MemberError::NotFound(format!("member {id}"))),
    }
}

/// A non-archived member by id, else by display name (case-insensitive, exact). The error
/// lists the known names so the model can ask the user (§7).
pub async fn resolve(ctx: &PluginContext, key: &str) -> Result<Member, MemberError> {
    let key = key.trim();
    let members = list_members(ctx).await?;
    if let Some(m) = members.iter().find(|m| m.id == key) {
        return Ok(m.clone());
    }
    let lower = key.to_lowercase();
    if let Some(m) = members
        .iter()
        .find(|m| m.profile.display_name.to_lowercase() == lower)
    {
        return Ok(m.clone());
    }
    let names: Vec<&str> = members
        .iter()
        .map(|m| m.profile.display_name.as_str())
        .collect();
    Err(MemberError::NotFound(format!(
        "no member {key:?}; members: {}",
        if names.is_empty() {
            "none".to_string()
        } else {
            names.join(", ")
        }
    )))
}

async fn name_taken(
    ctx: &PluginContext,
    name: &str,
    except_id: Option<&str>,
) -> Result<bool, PluginError> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM multimember_members
         WHERE archived_at IS NULL AND lower(display_name) = lower($1) AND id <> $2",
    )
    .bind(name)
    .bind(except_id.unwrap_or(""))
    .fetch_optional(&ctx.db)
    .await?;
    Ok(row.is_some())
}

/// Adds a member. `attendance` gives their weekly defaults; `None` means dinner every day
/// (§4).
pub async fn add_member(
    ctx: &PluginContext,
    profile: &MemberProfile,
    attendance: Option<&[DefaultSlot]>,
) -> Result<Member, MemberError> {
    let today = ctx.today().await?;
    let profile = profile.normalized(today)?;
    if list_members(ctx).await?.len() >= MAX_MEMBERS {
        return Err(MemberRule::TooManyMembers { max: MAX_MEMBERS }.into());
    }
    if name_taken(ctx, &profile.display_name, None).await? {
        return Err(MemberRule::NameTaken.into());
    }
    let default_dinners: Vec<DefaultSlot> = (0..7)
        .map(|weekday| DefaultSlot {
            weekday,
            meal_type: MealType::Dinner,
            present: true,
        })
        .collect();
    let slots = attendance.unwrap_or(&default_dinners);
    for slot in slots {
        check_weekday(slot.weekday)?;
    }

    let id = Uuid::now_v7().to_string();
    let now = ctx.clock.now();
    let mut tx = ctx.db.begin().await?;
    sqlx::query(
        "INSERT INTO multimember_members (id, display_name, sex, birth_date, height_cm, weight_kg,
             activity_level, dietary_pattern, allergies_json, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $10)",
    )
    .bind(&id)
    .bind(&profile.display_name)
    .bind(enum_str(&profile.sex))
    .bind(profile.birth_date.to_string())
    .bind(profile.height_cm)
    .bind(profile.weight_kg)
    .bind(enum_str(&profile.activity_level))
    .bind(enum_str(&profile.dietary_pattern))
    .bind(allergies_json(&profile)?)
    .bind(now.to_rfc3339())
    .execute(&mut *tx)
    .await?;
    for slot in slots {
        upsert_default(&mut *tx, &id, slot).await?;
    }
    tx.commit().await?;
    Ok(Member {
        id,
        profile,
        created_at: now,
        updated_at: now,
    })
}

fn allergies_json(profile: &MemberProfile) -> Result<String, PluginError> {
    serde_json::to_string(&profile.allergies).map_err(|e| PluginError::Other(e.to_string()))
}

/// Replaces a member's profile.
pub async fn update_member(
    ctx: &PluginContext,
    id: &str,
    profile: &MemberProfile,
) -> Result<Member, MemberError> {
    let existing = get_member(ctx, id).await?;
    let today = ctx.today().await?;
    let profile = profile.normalized(today)?;
    if name_taken(ctx, &profile.display_name, Some(id)).await? {
        return Err(MemberRule::NameTaken.into());
    }
    let now = ctx.clock.now();
    sqlx::query(
        "UPDATE multimember_members SET display_name = $2, sex = $3, birth_date = $4,
             height_cm = $5, weight_kg = $6, activity_level = $7, dietary_pattern = $8,
             allergies_json = $9, updated_at = $10
         WHERE id = $1",
    )
    .bind(id)
    .bind(&profile.display_name)
    .bind(enum_str(&profile.sex))
    .bind(profile.birth_date.to_string())
    .bind(profile.height_cm)
    .bind(profile.weight_kg)
    .bind(enum_str(&profile.activity_level))
    .bind(enum_str(&profile.dietary_pattern))
    .bind(allergies_json(&profile)?)
    .bind(now.to_rfc3339())
    .execute(&ctx.db)
    .await?;
    Ok(Member {
        profile,
        updated_at: now,
        ..existing
    })
}

/// Hides a member from lists and plans. Stored plans keep their name (§6.3).
pub async fn archive_member(ctx: &PluginContext, id: &str) -> Result<(), MemberError> {
    let done = sqlx::query(
        "UPDATE multimember_members SET archived_at = $2 WHERE id = $1 AND archived_at IS NULL",
    )
    .bind(id)
    .bind(ctx.clock.now().to_rfc3339())
    .execute(&ctx.db)
    .await?;
    if done.rows_affected() == 0 {
        return Err(MemberError::NotFound(format!("member {id}")));
    }
    Ok(())
}

// ---- Attendance -------------------------------------------------------------------------------

fn check_weekday(weekday: u8) -> Result<(), MemberRule> {
    if weekday > 6 {
        return Err(MemberRule::AttendanceInvalid);
    }
    Ok(())
}

async fn upsert_default<'e, E>(
    db: E,
    member_id: &str,
    slot: &DefaultSlot,
) -> Result<(), sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query(
        "INSERT INTO multimember_attendance_defaults (member_id, weekday, meal_type, present)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (member_id, weekday, meal_type) DO UPDATE SET present = EXCLUDED.present",
    )
    .bind(member_id)
    .bind(i64::from(slot.weekday))
    .bind(slot.meal_type.as_str())
    .bind(i64::from(slot.present))
    .execute(db)
    .await?;
    Ok(())
}

/// Sets one weekly default for a member.
pub async fn set_default(
    ctx: &PluginContext,
    member_id: &str,
    slot: &DefaultSlot,
) -> Result<(), MemberError> {
    check_weekday(slot.weekday)?;
    get_member(ctx, member_id).await?;
    upsert_default(&ctx.db, member_id, slot).await?;
    Ok(())
}

/// A member's weekly defaults: one entry per weekday and main meal, absent where unset.
pub async fn defaults(
    ctx: &PluginContext,
    member_id: &str,
) -> Result<Vec<DefaultSlot>, PluginError> {
    let rows: Vec<(i64, String, i64)> = sqlx::query_as(
        "SELECT weekday, meal_type, present FROM multimember_attendance_defaults
         WHERE member_id = $1",
    )
    .bind(member_id)
    .fetch_all(&ctx.db)
    .await?;
    let mut stored = BTreeMap::new();
    for (weekday, meal, present) in rows {
        stored.insert((weekday, parse_meal(&meal)?), present != 0);
    }
    let mut slots = Vec::with_capacity(21);
    for weekday in 0..7u8 {
        for meal_type in MealType::ALL {
            slots.push(DefaultSlot {
                weekday,
                meal_type,
                present: stored
                    .get(&(i64::from(weekday), meal_type))
                    .copied()
                    .unwrap_or(false),
            });
        }
    }
    Ok(slots)
}

/// Sets a one-off attendance for `date` (today or later).
pub async fn set_override(
    ctx: &PluginContext,
    member_id: &str,
    date: NaiveDate,
    meal_type: MealType,
    present: bool,
    note: Option<&str>,
) -> Result<(), MemberError> {
    let note = note.map(str::trim).filter(|n| !n.is_empty());
    if date < ctx.today().await? || note.is_some_and(|n| n.chars().count() > MAX_NOTE_CHARS) {
        return Err(MemberRule::AttendanceInvalid.into());
    }
    get_member(ctx, member_id).await?;
    sqlx::query(
        "INSERT INTO multimember_attendance_overrides
             (member_id, plan_date, meal_type, present, note, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (member_id, plan_date, meal_type) DO UPDATE
         SET present = EXCLUDED.present, note = EXCLUDED.note, updated_at = EXCLUDED.updated_at",
    )
    .bind(member_id)
    .bind(date.to_string())
    .bind(meal_type.as_str())
    .bind(i64::from(present))
    .bind(note)
    .bind(ctx.clock.now().to_rfc3339())
    .execute(&ctx.db)
    .await?;
    Ok(())
}

/// Removes a one-off attendance, so the weekly default applies again.
pub async fn clear_override(
    ctx: &PluginContext,
    member_id: &str,
    date: NaiveDate,
    meal_type: MealType,
) -> Result<(), MemberError> {
    let done = sqlx::query(
        "DELETE FROM multimember_attendance_overrides
         WHERE member_id = $1 AND plan_date = $2 AND meal_type = $3",
    )
    .bind(member_id)
    .bind(date.to_string())
    .bind(meal_type.as_str())
    .execute(&ctx.db)
    .await?;
    if done.rows_affected() == 0 {
        return Err(MemberError::NotFound(format!(
            "no attendance override for {date} {}",
            meal_type.as_str()
        )));
    }
    Ok(())
}

/// Overrides of non-archived members between `from` and `to`, inclusive, by date.
pub async fn overrides_between(
    ctx: &PluginContext,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<AttendanceOverride>, PluginError> {
    let rows: Vec<(String, String, String, i64, Option<String>)> = sqlx::query_as(
        "SELECT o.member_id, o.plan_date, o.meal_type, o.present, o.note
         FROM multimember_attendance_overrides o
         JOIN multimember_members m ON m.id = o.member_id AND m.archived_at IS NULL
         WHERE o.plan_date BETWEEN $1 AND $2
         ORDER BY o.plan_date, m.seq",
    )
    .bind(from.to_string())
    .bind(to.to_string())
    .fetch_all(&ctx.db)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for (member_id, date, meal, present, note) in rows {
        out.push(AttendanceOverride {
            member_id,
            plan_date: parse_date(&date)?,
            meal_type: parse_meal(&meal)?,
            present: present != 0,
            note,
        });
    }
    out.sort_by_key(|o| (o.plan_date, o.meal_type));
    Ok(out)
}

/// Who eats each main meal on `date`: override, else weekly default, else absent (§4). Member
/// ids are in the order the members were added; meals nobody attends are left out.
pub async fn attendance_on(
    ctx: &PluginContext,
    date: NaiveDate,
) -> Result<BTreeMap<MealType, Vec<String>>, PluginError> {
    let weekday = i64::from(date.weekday().num_days_from_monday());
    let rows: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT m.id, meal.meal_type,
                COALESCE(o.present, d.present, 0) AS present
         FROM multimember_members m
         CROSS JOIN (VALUES ('breakfast'), ('lunch'), ('dinner')) AS meal(meal_type)
         LEFT JOIN multimember_attendance_defaults d
           ON d.member_id = m.id AND d.weekday = $1 AND d.meal_type = meal.meal_type
         LEFT JOIN multimember_attendance_overrides o
           ON o.member_id = m.id AND o.plan_date = $2 AND o.meal_type = meal.meal_type
         WHERE m.archived_at IS NULL
         ORDER BY m.seq",
    )
    .bind(weekday)
    .bind(date.to_string())
    .fetch_all(&ctx.db)
    .await?;
    let mut out: BTreeMap<MealType, Vec<String>> = BTreeMap::new();
    for (member_id, meal, present) in rows {
        if present != 0 {
            out.entry(parse_meal(&meal)?).or_default().push(member_id);
        }
    }
    Ok(out)
}

// ---- Dislikes ---------------------------------------------------------------------------------

/// Adds a dislike. Without dates it is open-ended.
pub async fn add_dislike(
    ctx: &PluginContext,
    member_id: &str,
    text: &str,
    valid_from: Option<NaiveDate>,
    valid_until: Option<NaiveDate>,
) -> Result<Dislike, MemberError> {
    let text = text.trim();
    let len = text.chars().count();
    if len == 0 {
        return Err(MemberRule::DislikeEmpty.into());
    }
    if len > MAX_DISLIKE_CHARS {
        return Err(MemberRule::DislikeTooLong {
            got: len,
            max: MAX_DISLIKE_CHARS,
        }
        .into());
    }
    if let (Some(from), Some(until)) = (valid_from, valid_until) {
        if from > until {
            return Err(MemberRule::DatesReversed {
                valid_from: from,
                valid_until: until,
            }
            .into());
        }
    }
    get_member(ctx, member_id).await?;
    let id = Uuid::now_v7().to_string();
    let now = ctx.clock.now();
    sqlx::query(
        "INSERT INTO multimember_dislikes (id, member_id, text, valid_from, valid_until, created_at)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(&id)
    .bind(member_id)
    .bind(text)
    .bind(valid_from.map(|d| d.to_string()))
    .bind(valid_until.map(|d| d.to_string()))
    .bind(now.to_rfc3339())
    .execute(&ctx.db)
    .await?;
    Ok(Dislike {
        id,
        member_id: member_id.to_string(),
        text: text.to_string(),
        valid_from,
        valid_until,
        created_at: now,
    })
}

pub async fn remove_dislike(ctx: &PluginContext, id: &str) -> Result<(), MemberError> {
    let done = sqlx::query(
        "UPDATE multimember_dislikes SET deleted_at = $2 WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(id)
    .bind(ctx.clock.now().to_rfc3339())
    .execute(&ctx.db)
    .await?;
    if done.rows_affected() == 0 {
        return Err(MemberError::NotFound(format!("dislike {id}")));
    }
    Ok(())
}

/// id, text, valid_from, valid_until, created_at.
type DislikeRow = (String, String, Option<String>, Option<String>, String);

/// A member's dislikes, oldest first, expired ones included.
pub async fn dislikes(ctx: &PluginContext, member_id: &str) -> Result<Vec<Dislike>, PluginError> {
    let rows: Vec<DislikeRow> = sqlx::query_as(
        "SELECT id, text, valid_from, valid_until, created_at FROM multimember_dislikes
         WHERE member_id = $1 AND deleted_at IS NULL ORDER BY seq",
    )
    .bind(member_id)
    .fetch_all(&ctx.db)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for (id, text, from, until, created_at) in rows {
        out.push(Dislike {
            id,
            member_id: member_id.to_string(),
            text,
            valid_from: from.as_deref().map(parse_date).transpose()?,
            valid_until: until.as_deref().map(parse_date).transpose()?,
            created_at: parse_instant(&created_at)?,
        });
    }
    Ok(out)
}

/// The texts of a member's dislikes active on `date`.
pub async fn dislikes_on(
    ctx: &PluginContext,
    member_id: &str,
    date: NaiveDate,
) -> Result<Vec<String>, PluginError> {
    Ok(dislikes(ctx, member_id)
        .await?
        .into_iter()
        .filter(|d| d.is_active_on(date))
        .map(|d| d.text)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{aki, env, today};

    #[tokio::test]
    async fn add_list_update_archive() {
        let env = env().await;
        let ctx = &env.ctx;
        let m = add_member(ctx, &aki(), None).await.unwrap();
        assert_eq!(m.profile.display_name, "Aki");
        assert_eq!(list_members(ctx).await.unwrap(), vec![m.clone()]);

        let renamed = MemberProfile {
            display_name: "Aki M".into(),
            weight_kg: 54.0,
            ..aki()
        };
        let updated = update_member(ctx, &m.id, &renamed).await.unwrap();
        assert_eq!(get_member(ctx, &m.id).await.unwrap(), updated);

        archive_member(ctx, &m.id).await.unwrap();
        assert!(list_members(ctx).await.unwrap().is_empty());
        assert!(matches!(
            get_member(ctx, &m.id).await,
            Err(MemberError::NotFound(_))
        ));
        assert!(matches!(
            archive_member(ctx, &m.id).await,
            Err(MemberError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn names_are_unique_case_insensitively_among_active_members() {
        let env = env().await;
        let ctx = &env.ctx;
        let m = add_member(ctx, &aki(), None).await.unwrap();
        let shout = MemberProfile {
            display_name: "AKI".into(),
            ..aki()
        };
        assert!(matches!(
            add_member(ctx, &shout, None).await,
            Err(MemberError::Invalid(MemberRule::NameTaken))
        ));
        // Updating a member to its own name is fine; archiving frees the name.
        update_member(ctx, &m.id, &shout).await.unwrap();
        archive_member(ctx, &m.id).await.unwrap();
        add_member(ctx, &aki(), None).await.unwrap();
    }

    #[tokio::test]
    async fn at_most_seven_members() {
        let env = env().await;
        let ctx = &env.ctx;
        for i in 0..MAX_MEMBERS {
            let p = MemberProfile {
                display_name: format!("m{i}"),
                ..aki()
            };
            add_member(ctx, &p, None).await.unwrap();
        }
        assert!(matches!(
            add_member(ctx, &aki(), None).await,
            Err(MemberError::Invalid(MemberRule::TooManyMembers { max: 7 }))
        ));
    }

    #[tokio::test]
    async fn resolve_by_id_or_name_and_lists_names_when_unknown() {
        let env = env().await;
        let ctx = &env.ctx;
        let m = add_member(ctx, &aki(), None).await.unwrap();
        assert_eq!(resolve(ctx, &m.id).await.unwrap().id, m.id);
        assert_eq!(resolve(ctx, " aki ").await.unwrap().id, m.id);
        match resolve(ctx, "Ken").await {
            Err(MemberError::NotFound(msg)) => assert!(msg.contains("Aki"), "{msg}"),
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn new_members_default_to_dinner_every_day() {
        let env = env().await;
        let ctx = &env.ctx;
        let m = add_member(ctx, &aki(), None).await.unwrap();
        let slots = defaults(ctx, &m.id).await.unwrap();
        assert_eq!(slots.len(), 21);
        for s in slots {
            assert_eq!(s.present, s.meal_type == MealType::Dinner, "{s:?}");
        }
    }

    #[tokio::test]
    async fn attendance_resolves_override_then_default_then_absent() {
        let env = env().await;
        let ctx = &env.ctx;
        let day = today(ctx).await;
        let wd = day.weekday().num_days_from_monday() as u8;
        let m = add_member(
            ctx,
            &aki(),
            Some(&[DefaultSlot {
                weekday: wd,
                meal_type: MealType::Lunch,
                present: true,
            }]),
        )
        .await
        .unwrap();

        let on = attendance_on(ctx, day).await.unwrap();
        assert_eq!(on.get(&MealType::Lunch), Some(&vec![m.id.clone()]));
        assert_eq!(on.get(&MealType::Dinner), None, "no default row: absent");

        set_override(ctx, &m.id, day, MealType::Lunch, false, Some("外食"))
            .await
            .unwrap();
        set_override(ctx, &m.id, day, MealType::Dinner, true, None)
            .await
            .unwrap();
        let on = attendance_on(ctx, day).await.unwrap();
        assert_eq!(on.get(&MealType::Lunch), None);
        assert_eq!(on.get(&MealType::Dinner), Some(&vec![m.id.clone()]));

        // The next week's same weekday only has the default.
        let next = attendance_on(ctx, day + chrono::Days::new(7))
            .await
            .unwrap();
        assert_eq!(next.get(&MealType::Lunch), Some(&vec![m.id.clone()]));

        clear_override(ctx, &m.id, day, MealType::Lunch)
            .await
            .unwrap();
        let on = attendance_on(ctx, day).await.unwrap();
        assert_eq!(on.get(&MealType::Lunch), Some(&vec![m.id.clone()]));
        let listed = overrides_between(ctx, day, day).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].meal_type, MealType::Dinner);

        // Archived members attend nothing.
        archive_member(ctx, &m.id).await.unwrap();
        assert!(attendance_on(ctx, day).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn attendance_rejects_past_dates_bad_weekdays_and_long_notes() {
        let env = env().await;
        let ctx = &env.ctx;
        let day = today(ctx).await;
        let m = add_member(ctx, &aki(), None).await.unwrap();
        let yesterday = day.pred_opt().unwrap();
        let invalid = |r| matches!(r, Err(MemberError::Invalid(MemberRule::AttendanceInvalid)));
        assert!(invalid(
            set_override(ctx, &m.id, yesterday, MealType::Dinner, true, None).await
        ));
        assert!(invalid(
            set_override(
                ctx,
                &m.id,
                day,
                MealType::Dinner,
                true,
                Some(&"长".repeat(MAX_NOTE_CHARS + 1))
            )
            .await
        ));
        let bad = DefaultSlot {
            weekday: 7,
            meal_type: MealType::Dinner,
            present: true,
        };
        assert!(invalid(set_default(ctx, &m.id, &bad).await));
        assert!(matches!(
            set_default(ctx, "nope", &DefaultSlot { weekday: 0, ..bad }).await,
            Err(MemberError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn dislikes_round_trip_with_dates() {
        let env = env().await;
        let ctx = &env.ctx;
        let day = today(ctx).await;
        let m = add_member(ctx, &aki(), None).await.unwrap();
        add_dislike(ctx, &m.id, " セロリ ", None, None)
            .await
            .unwrap();
        let later = add_dislike(ctx, &m.id, "納豆", Some(day.succ_opt().unwrap()), None)
            .await
            .unwrap();
        assert_eq!(dislikes_on(ctx, &m.id, day).await.unwrap(), vec!["セロリ"]);
        assert_eq!(dislikes(ctx, &m.id).await.unwrap().len(), 2);

        remove_dislike(ctx, &later.id).await.unwrap();
        assert_eq!(dislikes(ctx, &m.id).await.unwrap().len(), 1);
        assert!(matches!(
            remove_dislike(ctx, &later.id).await,
            Err(MemberError::NotFound(_))
        ));

        for (text, from, until, rule) in [
            ("  ", None, None, MemberRule::DislikeEmpty),
            (
                "x",
                Some(day.succ_opt().unwrap()),
                Some(day),
                MemberRule::DatesReversed {
                    valid_from: day.succ_opt().unwrap(),
                    valid_until: day,
                },
            ),
        ] {
            match add_dislike(ctx, &m.id, text, from, until).await {
                Err(MemberError::Invalid(r)) => assert_eq!(r, rule),
                other => panic!("{other:?}"),
            }
        }
        let long = "长".repeat(MAX_DISLIKE_CHARS + 1);
        assert!(matches!(
            add_dislike(ctx, &m.id, &long, None, None).await,
            Err(MemberError::Invalid(MemberRule::DislikeTooLong { .. }))
        ));
    }

    #[tokio::test]
    async fn member_age_is_checked_against_the_clock() {
        let env = env().await;
        let ctx = &env.ctx;
        let day = today(ctx).await;
        let too_old = MemberProfile {
            birth_date: day
                .checked_sub_months(chrono::Months::new(71 * 12))
                .unwrap(),
            ..aki()
        };
        assert!(matches!(
            add_member(ctx, &too_old, None).await,
            Err(MemberError::Invalid(MemberRule::AgeOutOfRange { got: 71 }))
        ));
    }
}
