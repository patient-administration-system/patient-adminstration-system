//! Appointment series — recurring appointments (v0.9).
//!
//! An [`AppointmentSeries`] is a *plan* that expands into concrete
//! [`crate::models::appointment::Appointment`] rows at create time. The
//! recurrence rule is a deliberately narrow subset of RFC 5545: `FREQ` of
//! `Daily` / `Weekly` / `Monthly`, an `INTERVAL` of 1+, an optional
//! weekly `BYDAY` set, and exactly one of `COUNT` or `UNTIL` for
//! termination. Yearly, sub-daily, `BYMONTHDAY`, etc. are explicitly
//! out of v0.9 scope.
//!
//! Each generated occurrence is a normal `Appointment` row with
//! `series_id = Some(<series.id>)`, `slot_id = None`, and status
//! `Booked`. Per-patient overlap checking still applies — series
//! creation is **atomic**: a single occurrence that conflicts rolls
//! back the whole transaction. Pair with `POST /api/appointment-
//! series/preview` to dry-run the datetimes before commit.

use chrono::{DateTime, Datelike, Days, Months, Utc, Weekday};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{Error, Result};

/// Hard cap on the number of occurrences any one series may carry. Set
/// to 200 because (a) a daily appointment for two years already hits
/// 730, which is far beyond clinically sensible; (b) the create path
/// runs N per-patient overlap queries and N insert statements in one
/// transaction — bounding N bounds blast radius.
pub const MAX_OCCURRENCES: u32 = 200;

/// Recurrence frequency. RFC 5545's `FREQ`, minus the variants v0.9
/// does not implement (yearly, hourly, minutely, secondly).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Frequency {
    Daily,
    Weekly,
    Monthly,
}

/// Termination condition for a recurrence rule. Exactly one of `Count`
/// or `Until` is present; never both, never neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RecurrenceEnd {
    /// Exactly N occurrences (including the first one at
    /// `series.start_datetime`). Must be `>= 1`.
    Count { count: u32 },
    /// Stop after this datetime (inclusive — occurrences whose start
    /// is `<= until` are kept). Must be `>= series.start_datetime`.
    Until { until: DateTime<Utc> },
}

/// One recurrence rule. Mirrors RFC 5545 fields but in struct form
/// rather than the iCal string syntax — we never round-trip to the
/// wire format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecurrenceRule {
    pub frequency: Frequency,
    /// Every N units of `frequency`. `1` = every day/week/month.
    /// Must be `>= 1`.
    pub interval: u32,
    /// For `Frequency::Weekly` only: restrict to these weekdays. When
    /// empty / `None`, the series fires on the same weekday as
    /// `start_datetime`. Ignored for Daily / Monthly.
    #[serde(default)]
    pub by_weekday: Option<Vec<Weekday>>,
    pub end: RecurrenceEnd,
}

impl RecurrenceRule {
    /// Lightweight validation. `compute_occurrences` re-validates and
    /// also enforces [`MAX_OCCURRENCES`].
    pub fn validate(&self) -> Result<()> {
        if self.interval == 0 {
            return Err(Error::validation("recurrence interval must be >= 1"));
        }
        match &self.end {
            RecurrenceEnd::Count { count } if *count == 0 => {
                Err(Error::validation("recurrence count must be >= 1"))
            }
            _ => Ok(()),
        }?;
        if let Some(days) = &self.by_weekday
            && self.frequency != Frequency::Weekly
            && !days.is_empty()
        {
            return Err(Error::validation(
                "by_weekday is only meaningful for Frequency::Weekly",
            ));
        }
        Ok(())
    }
}

/// Status of an appointment series. Individual occurrence statuses live
/// on the [`Appointment`] rows themselves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeriesStatus {
    Active,
    Cancelled,
}

/// The series record. A series row plus N appointment rows are written
/// in one DB transaction at create time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppointmentSeries {
    pub id: Uuid,
    pub patient_id: Uuid,
    pub practitioner_id: Option<Uuid>,
    pub service_type: String,
    /// First occurrence's start time. Subsequent occurrences are
    /// computed as offsets from this.
    pub start_datetime: DateTime<Utc>,
    /// Per-occurrence duration. Each generated appointment ends at
    /// `start + duration_minutes`.
    pub duration_minutes: u32,
    pub rule: RecurrenceRule,
    pub status: SeriesStatus,
    pub reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl AppointmentSeries {
    pub fn new(
        patient_id: Uuid,
        service_type: impl Into<String>,
        start_datetime: DateTime<Utc>,
        duration_minutes: u32,
        rule: RecurrenceRule,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            patient_id,
            practitioner_id: None,
            service_type: service_type.into(),
            start_datetime,
            duration_minutes,
            rule,
            status: SeriesStatus::Active,
            reason: None,
            created_at: now,
            updated_at: now,
        }
    }
}

/// Generate the concrete occurrence datetimes for a recurrence rule.
///
/// `start` is the first occurrence; subsequent ones step forward per
/// `rule.frequency` × `rule.interval` (with `by_weekday` filtering for
/// `Weekly`). For `Monthly`, the day-of-month is preserved where
/// possible; when the target month is shorter (e.g. Jan 31 → Feb 28),
/// the date is clamped to the last day of that month — same as
/// chrono's `Months` arithmetic.
///
/// Returns `Error::Validation` if the rule itself is malformed or if
/// the expanded series would exceed [`MAX_OCCURRENCES`]. The `Until`
/// variant terminates early once the next candidate would exceed
/// `until`.
pub fn compute_occurrences(
    rule: &RecurrenceRule,
    start: DateTime<Utc>,
) -> Result<Vec<DateTime<Utc>>> {
    rule.validate()?;

    // Cap-check `Count` up front so a 5000-count rule doesn't burn 5000
    // loop iterations before failing.
    if let RecurrenceEnd::Count { count } = &rule.end
        && *count > MAX_OCCURRENCES
    {
        return Err(Error::validation(format!(
            "recurrence count {count} exceeds MAX_OCCURRENCES ({MAX_OCCURRENCES})"
        )));
    }
    if let RecurrenceEnd::Until { until } = &rule.end
        && *until < start
    {
        return Err(Error::validation(
            "recurrence `until` must be >= series start_datetime",
        ));
    }

    let mut out: Vec<DateTime<Utc>> = Vec::new();
    let mut cursor = start;
    let interval = rule.interval;

    // The weekday filter for Weekly: when omitted/empty, the rule fires
    // only on the start weekday; when populated, fires on any matching
    // weekday within each interval-grouped week.
    let weekly_days: Vec<Weekday> = match (&rule.frequency, &rule.by_weekday) {
        (Frequency::Weekly, Some(d)) if !d.is_empty() => d.clone(),
        (Frequency::Weekly, _) => vec![start.weekday()],
        _ => vec![],
    };

    let target = |n: usize| -> bool {
        match &rule.end {
            RecurrenceEnd::Count { count } => n < *count as usize,
            RecurrenceEnd::Until { .. } => true,
        }
    };
    let within_until = |dt: DateTime<Utc>| -> bool {
        match &rule.end {
            RecurrenceEnd::Until { until } => dt <= *until,
            RecurrenceEnd::Count { .. } => true,
        }
    };

    match rule.frequency {
        Frequency::Daily => {
            while target(out.len()) && within_until(cursor) {
                out.push(cursor);
                if out.len() as u32 > MAX_OCCURRENCES {
                    return Err(Error::validation(format!(
                        "expanded recurrence exceeds MAX_OCCURRENCES ({MAX_OCCURRENCES})"
                    )));
                }
                cursor = cursor + Days::new(interval as u64);
            }
        }
        Frequency::Weekly => {
            // Walk day-by-day; on each step, if the weekday is in
            // `weekly_days` AND (days_since_start / 7) is a multiple of
            // interval, emit. This is the simplest correct
            // implementation; perf is fine at MAX_OCCURRENCES bound.
            let mut day_cursor = start;
            let start_week_offset: i64 =
                start.date_naive().iso_week().week() as i64 + (start.year() as i64) * 53; // monotonic week index proxy
            while target(out.len()) && within_until(day_cursor) {
                let this_week_offset: i64 = day_cursor.date_naive().iso_week().week() as i64
                    + (day_cursor.year() as i64) * 53;
                let weeks_since_start = this_week_offset - start_week_offset;
                if weeks_since_start >= 0
                    && weeks_since_start % interval as i64 == 0
                    && weekly_days.contains(&day_cursor.weekday())
                    && day_cursor >= start
                {
                    out.push(day_cursor);
                    if out.len() as u32 > MAX_OCCURRENCES {
                        return Err(Error::validation(format!(
                            "expanded recurrence exceeds MAX_OCCURRENCES ({MAX_OCCURRENCES})"
                        )));
                    }
                }
                day_cursor = day_cursor + Days::new(1);
                // Safety brake — bounded by MAX_OCCURRENCES * 7 *
                // interval at worst. For interval=1, daily-by-week, this
                // is 1400 iterations; tiny.
                if (day_cursor - start).num_days()
                    > (MAX_OCCURRENCES as i64) * 7 * (interval as i64)
                {
                    break;
                }
            }
        }
        Frequency::Monthly => {
            while target(out.len()) && within_until(cursor) {
                out.push(cursor);
                if out.len() as u32 > MAX_OCCURRENCES {
                    return Err(Error::validation(format!(
                        "expanded recurrence exceeds MAX_OCCURRENCES ({MAX_OCCURRENCES})"
                    )));
                }
                // chrono's `Months` arithmetic clamps to month-end when
                // the target month is shorter (Jan 31 + 1 month → Feb
                // 28/29) — exactly the semantics we want.
                cursor = match cursor.checked_add_months(Months::new(interval)) {
                    Some(next) => next,
                    None => break,
                };
            }
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn dt(y: i32, m: u32, d: u32, h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, 0, 0).unwrap()
    }

    #[test]
    fn test_compute_occurrences_daily_count_5() {
        let rule = RecurrenceRule {
            frequency: Frequency::Daily,
            interval: 1,
            by_weekday: None,
            end: RecurrenceEnd::Count { count: 5 },
        };
        let out = compute_occurrences(&rule, dt(2026, 6, 1, 9)).unwrap();
        assert_eq!(out.len(), 5);
        assert_eq!(out[0], dt(2026, 6, 1, 9));
        assert_eq!(out[4], dt(2026, 6, 5, 9));
    }

    #[test]
    fn test_compute_occurrences_daily_interval_3() {
        let rule = RecurrenceRule {
            frequency: Frequency::Daily,
            interval: 3,
            by_weekday: None,
            end: RecurrenceEnd::Count { count: 3 },
        };
        let out = compute_occurrences(&rule, dt(2026, 6, 1, 9)).unwrap();
        assert_eq!(
            out,
            vec![dt(2026, 6, 1, 9), dt(2026, 6, 4, 9), dt(2026, 6, 7, 9)]
        );
    }

    #[test]
    fn test_compute_occurrences_weekly_default_same_weekday() {
        // 2026-06-01 is a Monday. Weekly with no by_weekday should fire
        // every Monday.
        let rule = RecurrenceRule {
            frequency: Frequency::Weekly,
            interval: 1,
            by_weekday: None,
            end: RecurrenceEnd::Count { count: 4 },
        };
        let out = compute_occurrences(&rule, dt(2026, 6, 1, 9)).unwrap();
        assert_eq!(out.len(), 4);
        for d in &out {
            assert_eq!(d.weekday(), Weekday::Mon);
        }
        assert_eq!(out[3], dt(2026, 6, 22, 9));
    }

    #[test]
    fn test_compute_occurrences_weekly_by_weekday_mwf() {
        // 2026-06-01 is a Monday. Weekly Mon/Wed/Fri, count=6 → two weeks
        // of MWF appointments.
        let rule = RecurrenceRule {
            frequency: Frequency::Weekly,
            interval: 1,
            by_weekday: Some(vec![Weekday::Mon, Weekday::Wed, Weekday::Fri]),
            end: RecurrenceEnd::Count { count: 6 },
        };
        let out = compute_occurrences(&rule, dt(2026, 6, 1, 9)).unwrap();
        assert_eq!(out.len(), 6);
        assert_eq!(
            out,
            vec![
                dt(2026, 6, 1, 9),  // Mon
                dt(2026, 6, 3, 9),  // Wed
                dt(2026, 6, 5, 9),  // Fri
                dt(2026, 6, 8, 9),  // Mon
                dt(2026, 6, 10, 9), // Wed
                dt(2026, 6, 12, 9), // Fri
            ]
        );
    }

    #[test]
    fn test_compute_occurrences_weekly_until_terminates() {
        let rule = RecurrenceRule {
            frequency: Frequency::Weekly,
            interval: 1,
            by_weekday: None,
            end: RecurrenceEnd::Until {
                until: dt(2026, 6, 14, 9),
            },
        };
        let out = compute_occurrences(&rule, dt(2026, 6, 1, 9)).unwrap();
        // Mondays 6/1, 6/8 (6/15 is past until).
        assert_eq!(out, vec![dt(2026, 6, 1, 9), dt(2026, 6, 8, 9)]);
    }

    #[test]
    fn test_compute_occurrences_monthly_clamps_to_short_month() {
        // Jan 31, +1 month → Feb 28 (2026 is not a leap year),
        // +1 month → Mar 28 (chrono preserves the clamped day).
        let rule = RecurrenceRule {
            frequency: Frequency::Monthly,
            interval: 1,
            by_weekday: None,
            end: RecurrenceEnd::Count { count: 3 },
        };
        let out = compute_occurrences(&rule, dt(2026, 1, 31, 9)).unwrap();
        assert_eq!(out[0], dt(2026, 1, 31, 9));
        assert_eq!(out[1], dt(2026, 2, 28, 9));
        // chrono's Months arithmetic preserves the *clamped* day,
        // not the original 31 — Feb 28 + 1 month = Mar 28.
        assert_eq!(out[2], dt(2026, 3, 28, 9));
    }

    #[test]
    fn test_compute_occurrences_rejects_zero_interval() {
        let rule = RecurrenceRule {
            frequency: Frequency::Daily,
            interval: 0,
            by_weekday: None,
            end: RecurrenceEnd::Count { count: 5 },
        };
        assert!(matches!(
            compute_occurrences(&rule, dt(2026, 6, 1, 9)),
            Err(Error::Validation(_))
        ));
    }

    #[test]
    fn test_compute_occurrences_rejects_zero_count() {
        let rule = RecurrenceRule {
            frequency: Frequency::Daily,
            interval: 1,
            by_weekday: None,
            end: RecurrenceEnd::Count { count: 0 },
        };
        assert!(matches!(
            compute_occurrences(&rule, dt(2026, 6, 1, 9)),
            Err(Error::Validation(_))
        ));
    }

    #[test]
    fn test_compute_occurrences_rejects_oversize_count() {
        let rule = RecurrenceRule {
            frequency: Frequency::Daily,
            interval: 1,
            by_weekday: None,
            end: RecurrenceEnd::Count {
                count: MAX_OCCURRENCES + 1,
            },
        };
        assert!(matches!(
            compute_occurrences(&rule, dt(2026, 6, 1, 9)),
            Err(Error::Validation(_))
        ));
    }

    #[test]
    fn test_compute_occurrences_rejects_until_before_start() {
        let rule = RecurrenceRule {
            frequency: Frequency::Daily,
            interval: 1,
            by_weekday: None,
            end: RecurrenceEnd::Until {
                until: dt(2026, 5, 30, 9),
            },
        };
        assert!(matches!(
            compute_occurrences(&rule, dt(2026, 6, 1, 9)),
            Err(Error::Validation(_))
        ));
    }

    #[test]
    fn test_compute_occurrences_rejects_by_weekday_on_daily() {
        let rule = RecurrenceRule {
            frequency: Frequency::Daily,
            interval: 1,
            by_weekday: Some(vec![Weekday::Mon]),
            end: RecurrenceEnd::Count { count: 3 },
        };
        assert!(matches!(
            compute_occurrences(&rule, dt(2026, 6, 1, 9)),
            Err(Error::Validation(_))
        ));
    }

    #[test]
    fn test_appointment_series_new_defaults_active() {
        let s = AppointmentSeries::new(
            Uuid::new_v4(),
            "cardiology",
            dt(2026, 6, 1, 9),
            30,
            RecurrenceRule {
                frequency: Frequency::Weekly,
                interval: 1,
                by_weekday: None,
                end: RecurrenceEnd::Count { count: 4 },
            },
        );
        assert_eq!(s.status, SeriesStatus::Active);
        assert_eq!(s.duration_minutes, 30);
        assert!(s.practitioner_id.is_none());
    }
}
