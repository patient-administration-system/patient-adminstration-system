//! rtt — Referral-to-Treatment pathway and clock arithmetic.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RTTStatus {
    Active,
    Paused,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RTTEventKind {
    Started,
    Paused,
    Resumed,
    Stopped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RTTPathway {
    pub id: Uuid,
    pub patient_id: Uuid,
    pub target_service: String,
    pub breach_weeks: u32,
    pub status: RTTStatus,
    pub started_at: DateTime<Utc>,
    pub stopped_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl RTTPathway {
    /// Default breach threshold in weeks (NHS RTT convention).
    pub const DEFAULT_BREACH_WEEKS: u32 = 18;

    pub fn new(patient_id: Uuid, target_service: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            patient_id,
            target_service: target_service.into(),
            breach_weeks: Self::DEFAULT_BREACH_WEEKS,
            status: RTTStatus::Active,
            started_at: now,
            stopped_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// True iff the active weeks waiting strictly exceeds `breach_weeks`.
    pub fn is_breaching(&self, events: &[RTTClockEvent], now: DateTime<Utc>) -> bool {
        compute_active_weeks(events, now) > self.breach_weeks
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RTTClockEvent {
    pub id: Uuid,
    pub pathway_id: Uuid,
    pub kind: RTTEventKind,
    pub reason: Option<String>,
    pub event_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

impl RTTClockEvent {
    pub fn started(pathway_id: Uuid) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            pathway_id,
            kind: RTTEventKind::Started,
            reason: None,
            event_at: now,
            created_at: now,
        }
    }

    pub fn paused(pathway_id: Uuid, reason: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            pathway_id,
            kind: RTTEventKind::Paused,
            reason: Some(reason.into()),
            event_at: now,
            created_at: now,
        }
    }

    pub fn resumed(pathway_id: Uuid) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            pathway_id,
            kind: RTTEventKind::Resumed,
            reason: None,
            event_at: now,
            created_at: now,
        }
    }

    pub fn stopped(pathway_id: Uuid, reason: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            pathway_id,
            kind: RTTEventKind::Stopped,
            reason: Some(reason.into()),
            event_at: now,
            created_at: now,
        }
    }
}

/// Sum the active intervals of an RTT clock and return the total in whole
/// weeks (floor).
///
/// Events MUST be sorted by `event_at` ascending. Returns 0 if `events` is
/// empty. The clock accrues time during intervals opened by `Started` or
/// `Resumed` and closed by `Paused` or `Stopped` (or, if the clock is still
/// open at the end of the list, by `now`).
pub fn compute_active_weeks(events: &[RTTClockEvent], now: DateTime<Utc>) -> u32 {
    if events.is_empty() {
        return 0;
    }

    let mut total_active_seconds: i64 = 0;
    let mut active_since: Option<DateTime<Utc>> = None;

    for event in events {
        match event.kind {
            RTTEventKind::Started | RTTEventKind::Resumed => {
                active_since = Some(event.event_at);
            }
            RTTEventKind::Paused | RTTEventKind::Stopped => {
                if let Some(start) = active_since.take() {
                    total_active_seconds += (event.event_at - start).num_seconds();
                }
            }
        }
    }

    if let Some(start) = active_since {
        total_active_seconds += (now - start).num_seconds();
    }

    const SECONDS_PER_WEEK: i64 = 7 * 24 * 3600;
    if total_active_seconds < 0 {
        return 0;
    }
    (total_active_seconds / SECONDS_PER_WEEK) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn at(now: DateTime<Utc>, weeks_ago: i64) -> DateTime<Utc> {
        now - Duration::weeks(weeks_ago)
    }

    fn event(pathway_id: Uuid, kind: RTTEventKind, event_at: DateTime<Utc>) -> RTTClockEvent {
        RTTClockEvent {
            id: Uuid::new_v4(),
            pathway_id,
            kind,
            reason: None,
            event_at,
            created_at: event_at,
        }
    }

    #[test]
    fn test_compute_active_weeks_empty_returns_zero() {
        let now = Utc::now();
        assert_eq!(compute_active_weeks(&[], now), 0);
    }

    #[test]
    fn test_compute_active_weeks_started_no_stop() {
        let pathway_id = Uuid::new_v4();
        let now = Utc::now();
        let events = vec![event(pathway_id, RTTEventKind::Started, at(now, 20))];
        assert_eq!(compute_active_weeks(&events, now), 20);
    }

    #[test]
    fn test_compute_active_weeks_started_then_paused_after_5_weeks() {
        let pathway_id = Uuid::new_v4();
        let now = Utc::now();
        let started_at = at(now, 10);
        let paused_at = started_at + Duration::weeks(5);
        let events = vec![
            event(pathway_id, RTTEventKind::Started, started_at),
            event(pathway_id, RTTEventKind::Paused, paused_at),
        ];
        assert_eq!(compute_active_weeks(&events, now), 5);
    }

    #[test]
    fn test_compute_active_weeks_pause_resume_seven_total() {
        // Started, run 2 weeks, Paused, 1 week gap, Resumed, run 5 weeks.
        // Total active = 7 weeks.
        let pathway_id = Uuid::new_v4();
        let now = Utc::now();
        let started_at = at(now, 20);
        let paused_at = started_at + Duration::weeks(2);
        let resumed_at = paused_at + Duration::weeks(1);
        let final_now = resumed_at + Duration::weeks(5);
        let events = vec![
            event(pathway_id, RTTEventKind::Started, started_at),
            event(pathway_id, RTTEventKind::Paused, paused_at),
            event(pathway_id, RTTEventKind::Resumed, resumed_at),
        ];
        assert_eq!(compute_active_weeks(&events, final_now), 7);
    }

    #[test]
    fn test_compute_active_weeks_started_paused_resumed_stopped() {
        // 3 weeks active, paused, resumed, 4 weeks active, stopped. Total 7.
        let pathway_id = Uuid::new_v4();
        let started_at = Utc::now() - Duration::weeks(30);
        let paused_at = started_at + Duration::weeks(3);
        let resumed_at = paused_at + Duration::weeks(2);
        let stopped_at = resumed_at + Duration::weeks(4);
        let events = vec![
            event(pathway_id, RTTEventKind::Started, started_at),
            event(pathway_id, RTTEventKind::Paused, paused_at),
            event(pathway_id, RTTEventKind::Resumed, resumed_at),
            event(pathway_id, RTTEventKind::Stopped, stopped_at),
        ];
        // `now` after the stop should not affect the total.
        let now = stopped_at + Duration::weeks(100);
        assert_eq!(compute_active_weeks(&events, now), 7);
    }

    #[test]
    fn test_is_breaching_true_at_20_weeks_with_threshold_18() {
        let now = Utc::now();
        let mut pathway = RTTPathway::new(Uuid::new_v4(), "cardiology");
        pathway.breach_weeks = 18;
        let events = vec![event(pathway.id, RTTEventKind::Started, at(now, 20))];
        assert!(pathway.is_breaching(&events, now));
    }

    #[test]
    fn test_is_breaching_false_at_5_weeks_with_threshold_18() {
        let now = Utc::now();
        let pathway = RTTPathway::new(Uuid::new_v4(), "cardiology");
        let events = vec![event(pathway.id, RTTEventKind::Started, at(now, 5))];
        assert!(!pathway.is_breaching(&events, now));
    }

    #[test]
    fn test_is_breaching_strictly_greater_than_threshold() {
        // At exactly 18 weeks, is_breaching must be false (uses `>`).
        let now = Utc::now();
        let pathway = RTTPathway::new(Uuid::new_v4(), "cardiology");
        let events = vec![event(pathway.id, RTTEventKind::Started, at(now, 18))];
        assert!(!pathway.is_breaching(&events, now));
    }

    #[test]
    fn test_rtt_pathway_new_defaults() {
        let patient_id = Uuid::new_v4();
        let p = RTTPathway::new(patient_id, "orthopedics");
        assert_eq!(p.patient_id, patient_id);
        assert_eq!(p.target_service, "orthopedics");
        assert_eq!(p.breach_weeks, 18);
        assert_eq!(p.status, RTTStatus::Active);
        assert!(p.stopped_at.is_none());
        assert_eq!(p.started_at, p.created_at);
        assert_eq!(p.created_at, p.updated_at);
    }

    #[test]
    fn test_rtt_clock_event_constructors() {
        let pathway_id = Uuid::new_v4();

        let s = RTTClockEvent::started(pathway_id);
        assert_eq!(s.kind, RTTEventKind::Started);
        assert_eq!(s.pathway_id, pathway_id);
        assert!(s.reason.is_none());

        let p = RTTClockEvent::paused(pathway_id, "patient request");
        assert_eq!(p.kind, RTTEventKind::Paused);
        assert_eq!(p.reason.as_deref(), Some("patient request"));

        let r = RTTClockEvent::resumed(pathway_id);
        assert_eq!(r.kind, RTTEventKind::Resumed);
        assert!(r.reason.is_none());

        let st = RTTClockEvent::stopped(pathway_id, "treated");
        assert_eq!(st.kind, RTTEventKind::Stopped);
        assert_eq!(st.reason.as_deref(), Some("treated"));
    }

    #[test]
    fn test_rtt_status_serde_roundtrip() {
        for s in [RTTStatus::Active, RTTStatus::Paused, RTTStatus::Stopped] {
            let json = serde_json::to_string(&s).expect("serialize");
            let back: RTTStatus = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, s);
        }
        assert_eq!(
            serde_json::to_string(&RTTStatus::Active).unwrap(),
            "\"active\""
        );
    }

    #[test]
    fn test_rtt_event_kind_serde_roundtrip() {
        for k in [
            RTTEventKind::Started,
            RTTEventKind::Paused,
            RTTEventKind::Resumed,
            RTTEventKind::Stopped,
        ] {
            let json = serde_json::to_string(&k).expect("serialize");
            let back: RTTEventKind = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, k);
        }
    }

    #[test]
    fn test_rtt_clock_event_serde_roundtrip() {
        let e = RTTClockEvent::paused(Uuid::new_v4(), "holiday");
        let json = serde_json::to_string(&e).expect("serialize");
        let back: RTTClockEvent = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.id, e.id);
        assert_eq!(back.pathway_id, e.pathway_id);
        assert_eq!(back.kind, RTTEventKind::Paused);
        assert_eq!(back.reason.as_deref(), Some("holiday"));
    }

    #[test]
    fn test_rtt_pathway_serde_roundtrip() {
        let p = RTTPathway::new(Uuid::new_v4(), "ent");
        let json = serde_json::to_string(&p).expect("serialize");
        let back: RTTPathway = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.id, p.id);
        assert_eq!(back.patient_id, p.patient_id);
        assert_eq!(back.target_service, "ent");
        assert_eq!(back.breach_weeks, 18);
        assert_eq!(back.status, RTTStatus::Active);
        assert!(back.stopped_at.is_none());
    }
}
