//! Check scheduler: runs checks on configurable intervals.

use std::collections::HashMap;
use std::time::Duration;

use tokio::time;

use crate::alert::{AlertEvent, AlertManager};
use crate::check::{Check, CheckResult, run_check};

/// Scheduler that runs checks and feeds results into the alert manager.
pub struct CheckScheduler {
    checks: Vec<Check>,
    alert_manager: AlertManager,
    /// Results from the most recent run of each check.
    last_results: HashMap<String, CheckResult>,
}

impl CheckScheduler {
    pub fn new() -> Self {
        Self {
            checks: Vec::new(),
            alert_manager: AlertManager::new(),
            last_results: HashMap::new(),
        }
    }

    /// Add a check to the scheduler.
    pub fn add_check(&mut self, check: Check) {
        self.alert_manager.register(
            &check.id,
            &check.name,
            check.failure_threshold,
            check.recovery_threshold,
        );
        self.checks.push(check);
    }

    /// Remove a check by ID.
    pub fn remove_check(&mut self, id: &str) {
        self.checks.retain(|c| c.id != id);
    }

    /// Run all checks once and return any alert events.
    pub async fn run_all(&mut self) -> Vec<AlertEvent> {
        let mut events = Vec::new();
        for check in &self.checks {
            let result = run_check(check).await;
            self.last_results.insert(check.id.clone(), result.clone());
            if let Some(event) = self.alert_manager.process(&result) {
                events.push(event);
            }
        }
        events
    }

    /// Run checks in a loop at the given interval.
    /// Calls the callback for each alert event.
    pub async fn run_loop<F>(&mut self, interval: Duration, mut on_event: F)
    where
        F: FnMut(AlertEvent),
    {
        let mut ticker = time::interval(interval);
        loop {
            ticker.tick().await;
            let events = self.run_all().await;
            for event in events {
                on_event(event);
            }
        }
    }

    /// Get the last result for a check.
    pub fn last_result(&self, check_id: &str) -> Option<&CheckResult> {
        self.last_results.get(check_id)
    }

    /// Get all currently firing alerts.
    pub fn firing_alerts(&self) -> Vec<&crate::alert::Alert> {
        self.alert_manager.firing()
    }

    /// Number of registered checks.
    pub fn check_count(&self) -> usize {
        self.checks.len()
    }
}

impl Default for CheckScheduler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::CheckType;

    #[tokio::test]
    async fn run_all_checks() {
        let mut scheduler = CheckScheduler::new();
        scheduler.add_check(Check {
            id: "disk-root".into(),
            name: "Root disk".into(),
            check_type: CheckType::Disk {
                path: if cfg!(windows) {
                    "C:\\".into()
                } else {
                    "/".into()
                },
                min_free_percent: 10.0,
            },
            interval_secs: 60,
            timeout_secs: 5,
            failure_threshold: 3,
            recovery_threshold: 2,
        });

        let events = scheduler.run_all().await;
        // Disk check should pass (root exists), no alert events on first run
        assert!(scheduler.last_result("disk-root").is_some());
    }

    #[tokio::test]
    async fn scheduler_produces_alert_events() {
        let mut scheduler = CheckScheduler::new();
        scheduler.add_check(Check {
            id: "tcp-closed".into(),
            name: "Closed port".into(),
            check_type: CheckType::Tcp {
                host: "127.0.0.1".into(),
                port: 19999,
            },
            interval_secs: 1,
            timeout_secs: 1,
            failure_threshold: 2,
            recovery_threshold: 1,
        });

        // First run: failure -> Pending (event)
        let events = scheduler.run_all().await;
        assert_eq!(events.len(), 1);

        // Second run: failure -> Firing (event, threshold reached)
        let events = scheduler.run_all().await;
        assert_eq!(events.len(), 1);

        assert_eq!(scheduler.firing_alerts().len(), 1);
    }
}
