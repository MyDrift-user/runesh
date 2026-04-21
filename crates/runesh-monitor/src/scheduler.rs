//! Check scheduler: runs checks on configurable intervals.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::stream::{FuturesUnordered, StreamExt};
use tokio::time;

use crate::alert::{AlertEvent, AlertManager, AlertStore};
use crate::check::{Check, CheckResult, CheckRuntime, run_check_with};

/// Scheduler that runs checks and feeds results into the alert manager.
pub struct CheckScheduler {
    checks: Vec<Check>,
    alert_manager: Arc<Mutex<AlertManager>>,
    /// Results from the most recent run of each check.
    last_results: HashMap<String, CheckResult>,
    runtime: CheckRuntime,
    store: Option<Arc<dyn AlertStore>>,
}

impl CheckScheduler {
    pub fn new() -> Self {
        Self {
            checks: Vec::new(),
            alert_manager: Arc::new(Mutex::new(AlertManager::new())),
            last_results: HashMap::new(),
            runtime: CheckRuntime::default(),
            store: None,
        }
    }

    /// Build a scheduler with a shared runtime (injected HTTP client/policy).
    pub fn with_runtime(runtime: CheckRuntime) -> Self {
        Self {
            checks: Vec::new(),
            alert_manager: Arc::new(Mutex::new(AlertManager::new())),
            last_results: HashMap::new(),
            runtime,
            store: None,
        }
    }

    /// Attach an `AlertStore`. Persisted state is loaded immediately, and the
    /// scheduler will save after every run cycle.
    pub async fn with_store(mut self, store: Arc<dyn AlertStore>) -> Self {
        if let Ok(snapshot) = store.load().await
            && let Ok(mut mgr) = self.alert_manager.lock()
        {
            mgr.restore_from(snapshot);
        }
        self.store = Some(store);
        self
    }

    /// Add a check to the scheduler.
    pub fn add_check(&mut self, check: Check) {
        if let Ok(mut mgr) = self.alert_manager.lock() {
            mgr.register(
                &check.id,
                &check.name,
                check.failure_threshold,
                check.recovery_threshold,
            );
        }
        self.checks.push(check);
    }

    /// Remove a check by ID.
    pub fn remove_check(&mut self, id: &str) {
        self.checks.retain(|c| c.id != id);
    }

    /// Run all checks in parallel and return any alert events.
    ///
    /// A slow check cannot delay a fast one: each check runs on its own
    /// future polled via `FuturesUnordered`.
    pub async fn run_all(&mut self) -> Vec<AlertEvent> {
        let runtime = self.runtime.clone();
        let mut in_flight: FuturesUnordered<_> = self
            .checks
            .iter()
            .map(|check| {
                let rt = runtime.clone();
                let check = check.clone();
                async move { run_check_with(&check, &rt).await }
            })
            .collect();

        let mut events = Vec::new();
        while let Some(result) = in_flight.next().await {
            self.last_results
                .insert(result.check_id.clone(), result.clone());
            if let Ok(mut mgr) = self.alert_manager.lock()
                && let Some(event) = mgr.process(&result)
            {
                events.push(event);
            }
        }

        if let Some(store) = &self.store {
            let snap = self
                .alert_manager
                .lock()
                .map(|g| g.snapshot())
                .unwrap_or_default();
            let _ = store.save(&snap).await;
        }

        events
    }

    /// Run checks in a loop at the given interval.
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
    pub fn firing_alerts(&self) -> Vec<crate::alert::Alert> {
        self.alert_manager
            .lock()
            .map(|mgr| mgr.firing().into_iter().cloned().collect())
            .unwrap_or_default()
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
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

        let _events = scheduler.run_all().await;
        assert!(scheduler.last_result("disk-root").is_some());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
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

        let events = scheduler.run_all().await;
        assert_eq!(events.len(), 1);

        let events = scheduler.run_all().await;
        assert_eq!(events.len(), 1);

        assert_eq!(scheduler.firing_alerts().len(), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn slow_check_does_not_block_fast_check() {
        let mut scheduler = CheckScheduler::new();
        scheduler.add_check(Check {
            id: "slow".into(),
            name: "Slow".into(),
            check_type: CheckType::Tcp {
                host: "192.0.2.1".into(),
                port: 81,
            },
            interval_secs: 1,
            timeout_secs: 5,
            failure_threshold: 1,
            recovery_threshold: 1,
        });
        scheduler.add_check(Check {
            id: "fast".into(),
            name: "Fast".into(),
            check_type: CheckType::Tcp {
                host: "127.0.0.1".into(),
                port: 19999,
            },
            interval_secs: 1,
            timeout_secs: 1,
            failure_threshold: 1,
            recovery_threshold: 1,
        });

        let start = std::time::Instant::now();
        let _ = scheduler.run_all().await;
        let elapsed = start.elapsed();

        assert!(scheduler.last_result("fast").is_some());
        assert!(scheduler.last_result("slow").is_some());
        let fast_ms = scheduler.last_result("fast").unwrap().duration_ms;
        assert!(fast_ms < 2000, "fast check took {fast_ms}ms");
        assert!(elapsed < Duration::from_secs(8), "elapsed = {:?}", elapsed);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn alert_store_round_trip() {
        use crate::alert::FileAlertStore;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("alerts.json");

        let store = Arc::new(FileAlertStore::new(&path));
        let mut sched = CheckScheduler::new().with_store(store.clone()).await;
        sched.add_check(Check {
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

        let _ = sched.run_all().await;
        let _ = sched.run_all().await;
        assert_eq!(sched.firing_alerts().len(), 1);

        // New scheduler using same store: state must reload.
        let store2: Arc<dyn AlertStore> = store.clone();
        let mut sched2 = CheckScheduler::new().with_store(store2).await;
        sched2.add_check(Check {
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
        let firing = sched2.firing_alerts();
        assert_eq!(firing.len(), 1, "expected persisted firing alert");
    }
}
