#![deny(unsafe_code)]
//! Patch management: ring-based rollout, CVE correlation, maintenance windows.
//!
//! # CVE correlation
//!
//! This crate does not fetch CVE feeds. CVE identifiers attached to a `Patch`
//! are caller-supplied. Callers should fetch the OSV/NVD feeds themselves and
//! attach the resulting identifiers before inserting a patch. `Patch::new`
//! validates every entry in `cve_ids` against the format `CVE-YYYY-NNNN..`.

use std::str::FromStr;
use std::time::Duration;

use chrono::{DateTime, Datelike, NaiveTime, Utc, Weekday};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

/// A pending patch/update.
///
/// CVE correlation is caller-supplied. Use [`Patch::new`] to validate CVE IDs
/// on construction.
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

impl Patch {
    /// Create a new patch, validating CVE IDs against `^CVE-\d{4}-\d{4,7}$`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: String,
        name: String,
        version: String,
        severity: PatchSeverity,
        cve_ids: Vec<String>,
        affected_packages: Vec<String>,
        release_date: DateTime<Utc>,
        status: PatchStatus,
    ) -> Result<Self, PatchError> {
        for cve in &cve_ids {
            validate_cve_id(cve)?;
        }
        Ok(Self {
            id,
            name,
            version,
            severity,
            cve_ids,
            affected_packages,
            release_date,
            status,
        })
    }
}

/// Validate a CVE identifier. Accepts strings matching `^CVE-\d{4}-\d{4,7}$`.
pub fn validate_cve_id(cve: &str) -> Result<(), PatchError> {
    static CVE_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = CVE_RE.get_or_init(|| regex::Regex::new(r"^CVE-\d{4}-\d{4,7}$").unwrap());
    if re.is_match(cve) {
        Ok(())
    } else {
        Err(PatchError::InvalidCveId(cve.to_string()))
    }
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

/// Observed metrics for the current ring, used to gate advancement.
#[derive(Debug, Clone, Copy, Default)]
pub struct RolloutMetrics {
    pub failure_rate_percent: f32,
    pub elapsed_since_start: Duration,
}

/// A maintenance window, evaluated in its declared IANA timezone.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaintenanceWindow {
    pub name: String,
    pub days: Vec<Weekday>,
    pub start_time: NaiveTime,
    pub end_time: NaiveTime,
    /// IANA timezone name (e.g. `Europe/Zurich`, `America/Los_Angeles`).
    pub timezone: String,
}

impl MaintenanceWindow {
    /// Build a window, validating the timezone string.
    pub fn new(
        name: String,
        days: Vec<Weekday>,
        start_time: NaiveTime,
        end_time: NaiveTime,
        timezone: String,
    ) -> Result<Self, PatchError> {
        Tz::from_str(&timezone).map_err(|_| PatchError::InvalidTimezone(timezone.clone()))?;
        Ok(Self {
            name,
            days,
            start_time,
            end_time,
            timezone,
        })
    }

    /// Check if the given instant falls within this window, evaluated in the
    /// window's declared timezone.
    pub fn is_active_at(&self, instant: DateTime<Utc>) -> bool {
        let tz = match Tz::from_str(&self.timezone) {
            Ok(tz) => tz,
            Err(_) => return false,
        };
        let local = instant.with_timezone(&tz);
        let weekday = local.date_naive().weekday();
        let time = local.time();
        self.days.contains(&weekday) && time >= self.start_time && time < self.end_time
    }

    /// Check if the current time falls within this window.
    pub fn is_active_now(&self) -> bool {
        self.is_active_at(Utc::now())
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

    /// Advance to the next ring, gated by the current ring's soak window and
    /// abort threshold. Returns `Ok(true)` if advanced, `Ok(false)` if already
    /// on the last ring.
    pub fn advance_ring(&mut self, metrics: &RolloutMetrics) -> Result<bool, RolloutError> {
        let Some(current) = self.rings.get(self.current_ring) else {
            return Ok(false);
        };
        let soak = Duration::from_secs(current.soak_hours as u64 * 3600);
        if metrics.elapsed_since_start < soak {
            return Err(RolloutError::SoakNotElapsed {
                required: soak,
                elapsed: metrics.elapsed_since_start,
            });
        }
        if metrics.failure_rate_percent as f64 > current.abort_threshold_percent {
            return Err(RolloutError::ThresholdExceeded {
                threshold: current.abort_threshold_percent,
                observed: metrics.failure_rate_percent,
            });
        }
        if self.current_ring + 1 < self.rings.len() {
            self.current_ring += 1;
            Ok(true)
        } else {
            Ok(false)
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
    #[error("invalid CVE id: {0}")]
    InvalidCveId(String),
    #[error("invalid timezone: {0}")]
    InvalidTimezone(String),
}

#[derive(Debug, thiserror::Error)]
pub enum RolloutError {
    #[error("soak period not elapsed (required {required:?}, elapsed {elapsed:?})")]
    SoakNotElapsed {
        required: Duration,
        elapsed: Duration,
    },
    #[error("failure rate {observed}% exceeds threshold {threshold}%")]
    ThresholdExceeded { threshold: f64, observed: f32 },
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

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
    fn cve_id_validation() {
        assert!(validate_cve_id("CVE-2026-1234").is_ok());
        assert!(validate_cve_id("CVE-1999-0001").is_ok());
        assert!(validate_cve_id("CVE-2024-1234567").is_ok());
        assert!(validate_cve_id("CVE-20-12").is_err());
        assert!(validate_cve_id("not-a-cve").is_err());
        assert!(validate_cve_id("CVE-2026-abc").is_err());
    }

    #[test]
    fn patch_new_rejects_bad_cve() {
        let err = Patch::new(
            "p1".into(),
            "n".into(),
            "1".into(),
            PatchSeverity::Low,
            vec!["bogus".into()],
            vec![],
            Utc::now(),
            PatchStatus::Available,
        )
        .unwrap_err();
        assert!(matches!(err, PatchError::InvalidCveId(_)));
    }

    fn sample_window(tz: &str) -> MaintenanceWindow {
        MaintenanceWindow::new(
            "Weekend".into(),
            vec![Weekday::Sat, Weekday::Sun],
            NaiveTime::from_hms_opt(2, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(6, 0, 0).unwrap(),
            tz.into(),
        )
        .unwrap()
    }

    #[test]
    fn window_rejects_bad_timezone() {
        let err = MaintenanceWindow::new(
            "Bad".into(),
            vec![Weekday::Mon],
            NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(1, 0, 0).unwrap(),
            "Not/A_Zone".into(),
        )
        .unwrap_err();
        assert!(matches!(err, PatchError::InvalidTimezone(_)));
    }

    #[test]
    fn window_tz_aware_los_angeles() {
        let w = sample_window("America/Los_Angeles");
        // 2025-01-04 is a Saturday. 03:00 America/Los_Angeles = 11:00 UTC.
        let inside = Utc.with_ymd_and_hms(2025, 1, 4, 11, 0, 0).unwrap();
        assert!(w.is_active_at(inside));
        // Same instant is outside the window if the TZ were UTC (time 11:00 UTC
        // would fall outside 02:00-06:00). Here we verify active-in-LA.
        // 07:00 LA = 15:00 UTC (Saturday) is outside.
        let outside = Utc.with_ymd_and_hms(2025, 1, 4, 15, 0, 0).unwrap();
        assert!(!w.is_active_at(outside));
    }

    #[test]
    fn window_tz_aware_zurich() {
        let w = sample_window("Europe/Zurich");
        // 2025-01-04 04:00 Europe/Zurich = 03:00 UTC (winter, CET=UTC+1).
        let inside = Utc.with_ymd_and_hms(2025, 1, 4, 3, 0, 0).unwrap();
        assert!(w.is_active_at(inside));
        // 2025-01-04 07:00 Zurich = 06:00 UTC, just past end.
        let outside = Utc.with_ymd_and_hms(2025, 1, 4, 6, 0, 0).unwrap();
        assert!(!w.is_active_at(outside));
    }

    #[test]
    fn rollout_ring_advance_respects_soak_and_threshold() {
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
            window: sample_window("Europe/Zurich"),
            current_ring: 0,
        };

        // Not yet soaked.
        let err = plan
            .advance_ring(&RolloutMetrics {
                failure_rate_percent: 0.0,
                elapsed_since_start: Duration::from_secs(3600),
            })
            .unwrap_err();
        assert!(matches!(err, RolloutError::SoakNotElapsed { .. }));
        assert_eq!(plan.current_ring_name(), "test");

        // Threshold exceeded.
        let err = plan
            .advance_ring(&RolloutMetrics {
                failure_rate_percent: 10.0,
                elapsed_since_start: Duration::from_secs(25 * 3600),
            })
            .unwrap_err();
        assert!(matches!(err, RolloutError::ThresholdExceeded { .. }));

        // Healthy and soaked.
        assert!(
            plan.advance_ring(&RolloutMetrics {
                failure_rate_percent: 1.0,
                elapsed_since_start: Duration::from_secs(25 * 3600),
            })
            .unwrap()
        );
        assert_eq!(plan.current_ring_name(), "pilot");

        // Advance through remaining rings.
        assert!(
            plan.advance_ring(&RolloutMetrics {
                failure_rate_percent: 0.0,
                elapsed_since_start: Duration::from_secs(49 * 3600),
            })
            .unwrap()
        );
        assert_eq!(plan.current_ring_name(), "broad");
        assert!(
            !plan
                .advance_ring(&RolloutMetrics {
                    failure_rate_percent: 0.0,
                    elapsed_since_start: Duration::from_secs(1),
                })
                .unwrap()
        );
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
