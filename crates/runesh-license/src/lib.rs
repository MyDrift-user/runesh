//! Software license tracking.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

/// A software license entitlement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct License {
    pub id: String,
    pub product: String,
    pub vendor: String,
    pub license_type: LicenseType,
    /// Total seats/units purchased.
    pub total_seats: u32,
    /// Currently assigned seats.
    pub used_seats: u32,
    pub purchase_date: Option<NaiveDate>,
    pub expiry_date: Option<NaiveDate>,
    pub cost: Option<f64>,
    pub currency: Option<String>,
    pub license_key: Option<String>,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LicenseType {
    Perpetual,
    Subscription,
    PerUser,
    PerDevice,
    SiteLicense,
    OpenSource,
    Trial,
}

/// License assignment to a user or device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Assignment {
    pub license_id: String,
    pub assigned_to: String,
    pub assigned_at: DateTime<Utc>,
}

/// Utilization report for a license.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Utilization {
    pub license_id: String,
    pub product: String,
    pub total: u32,
    pub used: u32,
    pub available: u32,
    pub utilization_percent: f64,
}

impl License {
    pub fn available_seats(&self) -> u32 {
        self.total_seats.saturating_sub(self.used_seats)
    }

    pub fn utilization_percent(&self) -> f64 {
        if self.total_seats == 0 {
            return 0.0;
        }
        (self.used_seats as f64 / self.total_seats as f64) * 100.0
    }

    pub fn is_expired(&self) -> bool {
        matches!(self.expiry_date, Some(exp) if exp < Utc::now().date_naive())
    }

    pub fn is_over_licensed(&self) -> bool {
        self.utilization_percent() < 50.0 && self.total_seats > 5
    }

    pub fn is_under_licensed(&self) -> bool {
        self.used_seats > self.total_seats
    }

    pub fn to_utilization(&self) -> Utilization {
        Utilization {
            license_id: self.id.clone(),
            product: self.product.clone(),
            total: self.total_seats,
            used: self.used_seats,
            available: self.available_seats(),
            utilization_percent: self.utilization_percent(),
        }
    }
}

/// In-memory license store.
#[derive(Debug, Default)]
pub struct LicenseStore {
    licenses: std::collections::HashMap<String, License>,
    assignments: Vec<Assignment>,
}

impl LicenseStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, license: License) {
        self.licenses.insert(license.id.clone(), license);
    }

    pub fn get(&self, id: &str) -> Option<&License> {
        self.licenses.get(id)
    }

    pub fn assign(&mut self, license_id: &str, to: &str) -> Result<(), String> {
        let lic = self
            .licenses
            .get_mut(license_id)
            .ok_or_else(|| format!("license {license_id} not found"))?;
        if lic.used_seats >= lic.total_seats {
            return Err("no available seats".into());
        }
        lic.used_seats += 1;
        self.assignments.push(Assignment {
            license_id: license_id.into(),
            assigned_to: to.into(),
            assigned_at: Utc::now(),
        });
        Ok(())
    }

    pub fn revoke(&mut self, license_id: &str, from: &str) -> bool {
        if let Some(lic) = self.licenses.get_mut(license_id) {
            let before = self.assignments.len();
            self.assignments
                .retain(|a| !(a.license_id == license_id && a.assigned_to == from));
            if self.assignments.len() < before {
                lic.used_seats = lic.used_seats.saturating_sub(1);
                return true;
            }
        }
        false
    }

    /// Get all over-licensed products (low utilization).
    pub fn over_licensed(&self) -> Vec<&License> {
        self.licenses
            .values()
            .filter(|l| l.is_over_licensed())
            .collect()
    }

    /// Get all under-licensed products (more used than owned).
    pub fn under_licensed(&self) -> Vec<&License> {
        self.licenses
            .values()
            .filter(|l| l.is_under_licensed())
            .collect()
    }

    /// Get licenses expiring within N days.
    pub fn expiring_within(&self, days: i64) -> Vec<&License> {
        let cutoff = Utc::now().date_naive() + chrono::Duration::days(days);
        self.licenses.values().filter(|l| {
            matches!(l.expiry_date, Some(exp) if exp <= cutoff && exp >= Utc::now().date_naive())
        }).collect()
    }

    pub fn len(&self) -> usize {
        self.licenses.len()
    }
    pub fn is_empty(&self) -> bool {
        self.licenses.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_license() -> License {
        License {
            id: "lic-1".into(),
            product: "Office 365".into(),
            vendor: "Microsoft".into(),
            license_type: LicenseType::PerUser,
            total_seats: 50,
            used_seats: 30,
            purchase_date: None,
            expiry_date: Some(NaiveDate::from_ymd_opt(2027, 12, 31).unwrap()),
            cost: Some(12000.0),
            currency: Some("USD".into()),
            license_key: None,
            notes: String::new(),
        }
    }

    #[test]
    fn utilization() {
        let lic = sample_license();
        assert_eq!(lic.available_seats(), 20);
        assert!((lic.utilization_percent() - 60.0).abs() < 0.01);
        assert!(!lic.is_over_licensed());
        assert!(!lic.is_under_licensed());
    }

    #[test]
    fn over_licensed_detection() {
        let mut lic = sample_license();
        lic.total_seats = 100;
        lic.used_seats = 10;
        assert!(lic.is_over_licensed());
    }

    #[test]
    fn under_licensed_detection() {
        let mut lic = sample_license();
        lic.used_seats = 51;
        assert!(lic.is_under_licensed());
    }

    #[test]
    fn assign_and_revoke() {
        let mut store = LicenseStore::new();
        store.add(sample_license());
        store.assign("lic-1", "alice@example.com").unwrap();
        assert_eq!(store.get("lic-1").unwrap().used_seats, 31);

        store.revoke("lic-1", "alice@example.com");
        assert_eq!(store.get("lic-1").unwrap().used_seats, 30);
    }

    #[test]
    fn no_seats_available() {
        let mut store = LicenseStore::new();
        let mut lic = sample_license();
        lic.total_seats = 1;
        lic.used_seats = 1;
        store.add(lic);
        assert!(store.assign("lic-1", "user").is_err());
    }

    #[test]
    fn serialization() {
        let lic = sample_license();
        let json = serde_json::to_string(&lic).unwrap();
        let parsed: License = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.product, "Office 365");
    }
}
