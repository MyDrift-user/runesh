#![deny(unsafe_code)]
//! Patch management: ring-based rollout, CVE correlation, maintenance windows.

use chrono::{DateTime, Datelike, NaiveTime, Utc, Weekday};
use serde::{Deserialize, Serialize};

/// A pending patch/update.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Patch {
    pub id: String,
    pub name: String,
    pub version: String,
    pub severity: PatchSeverity,
    pub cve_ids: Vec<String>,
    pub affected_packages: Vec<String>,
    pub release_date: DateTime<Utc>,
    pub status: PatchStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PatchSeverity {
    Critical,
    High,
    Medium,
    Low,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatchStatus {
    Available,
    Approved,
    Scheduled,
    Installing,
    Installed,
    Failed,
    Skipped,
}

/// A rollout ring (test -> pilot -> broad).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RolloutRing {
    pub name: String,
    pub order: u32,
    /// Soak period in hours before advancing to next ring.
    pub soak_hours: u32,
    /// Target selector (device group, tag, etc.).
    pub target: String,
    /// Max failure percentage before aborting.
    pub abort_threshold_percent: f64,
}

/// A maintenance window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaintenanceWindow {
    pub name: String,
    pub days: Vec<Weekday>,
    pub start_time: NaiveTime,
    pub end_time: NaiveTime,
    pub timezone: String,
}

impl MaintenanceWindow {
    /// Check if the current time falls within this window (naive, ignores timezone).
    pub fn is_active_now(&self) -> bool {
        let now = Utc::now();
        let today = now.date_naive().weekday();
        let time = now.time();
        self.days.contains(&today) && time >= self.start_time && time < self.end_time
    }
}

/// Rollout plan for a set of patches.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RolloutPlan {
    pub patches: Vec<Patch>,
    pub rings: Vec<RolloutRing>,
    pub window: MaintenanceWindow,
    pub current_ring: usize,
}

impl RolloutPlan {
    pub fn affected_device_count(&self) -> usize {
        // Placeholder: real impl queries device inventory
        0
    }

    pub fn advance_ring(&mut self) -> bool {
        if self.current_ring + 1 < self.rings.len() {
            self.current_ring += 1;
            true
        } else {
            false
        }
    }

    pub fn current_ring_name(&self) -> &str {
        self.rings
            .get(self.current_ring)
            .map(|r| r.name.as_str())
            .unwrap_or("unknown")
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PatchError {
    #[error("patch not found: {0}")]
    NotFound(String),
    #[error("rollout aborted: {0}")]
    Aborted(String),
    #[error("not in maintenance window")]
    NotInWindow,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_serialization() {
        let p = Patch {
            id: "p1".into(),
            name: "Security Update".into(),
            version: "1.0.1".into(),
            severity: PatchSeverity::Critical,
            cve_ids: vec!["CVE-2026-1234".into()],
            affected_packages: vec!["openssl".into()],
            release_date: Utc::now(),
            status: PatchStatus::Available,
        };
        let json = serde_json::to_string(&p).unwrap();
        let parsed: Patch = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.severity, PatchSeverity::Critical);
    }

    #[test]
    fn rollout_ring_advance() {
        let mut plan = RolloutPlan {
            patches: vec![],
            rings: vec![
                RolloutRing {
                    name: "test".into(),
                    order: 0,
                    soak_hours: 24,
                    target: "env:test".into(),
                    abort_threshold_percent: 5.0,
                },
                RolloutRing {
                    name: "pilot".into(),
                    order: 1,
                    soak_hours: 48,
                    target: "env:pilot".into(),
                    abort_threshold_percent: 2.0,
                },
                RolloutRing {
                    name: "broad".into(),
                    order: 2,
                    soak_hours: 0,
                    target: "*".into(),
                    abort_threshold_percent: 1.0,
                },
            ],
            window: MaintenanceWindow {
                name: "Weekend".into(),
                days: vec![Weekday::Sat, Weekday::Sun],
                start_time: NaiveTime::from_hms_opt(2, 0, 0).unwrap(),
                end_time: NaiveTime::from_hms_opt(6, 0, 0).unwrap(),
                timezone: "Europe/Zurich".into(),
            },
            current_ring: 0,
        };
        assert_eq!(plan.current_ring_name(), "test");
        assert!(plan.advance_ring());
        assert_eq!(plan.current_ring_name(), "pilot");
        assert!(plan.advance_ring());
        assert_eq!(plan.current_ring_name(), "broad");
        assert!(!plan.advance_ring()); // no more rings
    }

    #[test]
    fn all_severities() {
        for s in [
            PatchSeverity::Critical,
            PatchSeverity::High,
            PatchSeverity::Medium,
            PatchSeverity::Low,
            PatchSeverity::None,
        ] {
            let json = serde_json::to_string(&s).unwrap();
            let parsed: PatchSeverity = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, s);
        }
    }
}
