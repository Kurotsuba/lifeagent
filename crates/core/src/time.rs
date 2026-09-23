//! Logical-date computation and `"HH:MM"` parsing, per `docs/PHASE1_SPEC.md` §4.
//!
//! "Today" for the app is not the calendar day; it is bounded by a configurable day
//! boundary (default 04:00) so a late-night snack still counts toward the day the user is
//! living, not the next one.

use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveDateTime, NaiveTime, Timelike, Utc};

use crate::error::CoreError;

/// `logical_date(instant, boundary) -> NaiveDate` from §4: convert to local time, subtract
/// the boundary's time-of-day, take the date. Delegates to [`logical_date_from_local`], which
/// is the actually-tested pure function — `chrono::Local` reads the process timezone, so a
/// test asserting on a specific date needs to bypass that and supply a local datetime
/// directly rather than depend on the machine's TZ.
pub fn logical_date(instant: DateTime<Utc>, boundary: NaiveTime) -> NaiveDate {
    logical_date_from_local(instant.with_timezone(&Local).naive_local(), boundary)
}

/// The pure part of [`logical_date`]: given a naive local datetime, shift it back by the
/// boundary's time-of-day and take the date. Anything before the boundary belongs to the
/// previous day.
pub fn logical_date_from_local(local: NaiveDateTime, boundary: NaiveTime) -> NaiveDate {
    let boundary_secs = boundary.num_seconds_from_midnight() as i64;
    (local - chrono::Duration::seconds(boundary_secs)).date()
}

/// Parses a `"HH:MM"` setting value (`plan_time`, `day_boundary`).
pub fn parse_hhmm(s: &str) -> Result<NaiveTime, CoreError> {
    NaiveTime::parse_from_str(s, "%H:%M")
        .map_err(|_| CoreError::Validation(format!("invalid HH:MM time: {s:?}")))
}

/// Formats a time as the `"HH:MM"` setting value.
pub fn format_hhmm(t: NaiveTime) -> String {
    t.format("%H:%M").to_string()
}

/// Age in whole years at `today`, for profile validation (§7.4).
pub fn age_years(birth_date: NaiveDate, today: NaiveDate) -> i32 {
    let mut age = today.year() - birth_date.year();
    let birthday_passed_this_year =
        (today.month(), today.day()) >= (birth_date.month(), birth_date.day());
    if !birthday_passed_this_year {
        age -= 1;
    }
    age
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn logical_date_before_boundary_is_previous_day() {
        // 03:59 local on the 10th, boundary 04:00 -> logical date 9th.
        let boundary = NaiveTime::from_hms_opt(4, 0, 0).unwrap();
        let local = NaiveDate::from_ymd_opt(2026, 9, 10)
            .unwrap()
            .and_hms_opt(3, 59, 0)
            .unwrap();
        assert_eq!(
            logical_date_from_local(local, boundary),
            NaiveDate::from_ymd_opt(2026, 9, 9).unwrap()
        );
    }

    #[test]
    fn logical_date_at_boundary_is_same_day() {
        // 04:00 local on the 10th, boundary 04:00 -> logical date 10th.
        let boundary = NaiveTime::from_hms_opt(4, 0, 0).unwrap();
        let local = NaiveDate::from_ymd_opt(2026, 9, 10)
            .unwrap()
            .and_hms_opt(4, 0, 0)
            .unwrap();
        assert_eq!(
            logical_date_from_local(local, boundary),
            NaiveDate::from_ymd_opt(2026, 9, 10).unwrap()
        );
    }

    #[test]
    fn logical_date_midday_is_same_day_regardless_of_boundary() {
        let boundary = NaiveTime::from_hms_opt(4, 0, 0).unwrap();
        let local = NaiveDate::from_ymd_opt(2026, 9, 10)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap();
        assert_eq!(
            logical_date_from_local(local, boundary),
            NaiveDate::from_ymd_opt(2026, 9, 10).unwrap()
        );
    }

    #[test]
    fn logical_date_public_fn_round_trips_through_local_timezone() {
        // Smoke-test the public `DateTime<Utc>` entry point once; the boundary-crossing
        // behavior itself is covered above on the pure local-time function.
        let boundary = NaiveTime::from_hms_opt(4, 0, 0).unwrap();
        let instant = Utc.with_ymd_and_hms(2026, 9, 10, 12, 0, 0).unwrap();
        let expected =
            logical_date_from_local(instant.with_timezone(&Local).naive_local(), boundary);
        assert_eq!(logical_date(instant, boundary), expected);
    }

    #[test]
    fn parse_hhmm_accepts_valid_time() {
        assert_eq!(
            parse_hhmm("21:00").unwrap(),
            NaiveTime::from_hms_opt(21, 0, 0).unwrap()
        );
        assert_eq!(
            parse_hhmm("04:00").unwrap(),
            NaiveTime::from_hms_opt(4, 0, 0).unwrap()
        );
    }

    #[test]
    fn parse_hhmm_rejects_garbage() {
        assert!(parse_hhmm("not-a-time").is_err());
        assert!(parse_hhmm("24:61").is_err());
    }

    #[test]
    fn format_hhmm_round_trips() {
        let t = NaiveTime::from_hms_opt(21, 5, 0).unwrap();
        assert_eq!(format_hhmm(t), "21:05");
        assert_eq!(parse_hhmm(&format_hhmm(t)).unwrap(), t);
    }

    #[test]
    fn age_years_before_birthday_this_year() {
        let birth = NaiveDate::from_ymd_opt(2000, 12, 31).unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 9, 23).unwrap();
        assert_eq!(age_years(birth, today), 25);
    }

    #[test]
    fn age_years_after_birthday_this_year() {
        let birth = NaiveDate::from_ymd_opt(2000, 1, 1).unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 9, 23).unwrap();
        assert_eq!(age_years(birth, today), 26);
    }

    #[test]
    fn age_years_on_birthday_counts_the_new_age() {
        let birth = NaiveDate::from_ymd_opt(2000, 9, 23).unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 9, 23).unwrap();
        assert_eq!(age_years(birth, today), 26);
    }
}
