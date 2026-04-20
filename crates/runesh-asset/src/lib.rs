//! Hardware and software asset tracking.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

/// A tracked hardware asset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareAsset {
    pub id: String,
    pub name: String,
    pub asset_tag: Option<String>,
    pub serial_number: Option<String>,
    pub model: String,
    pub vendor: String,
    pub category: AssetCategory,
    pub status: AssetStatus,
    pub purchase_date: Option<NaiveDate>,
    pub purchase_price: Option<f64>,
    pub currency: Option<String>,
    pub warranty_expires: Option<NaiveDate>,
    pub invoice_ref: Option<String>,
    #[serde(default)]
    pub assigned_to: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub notes: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Asset categories.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetCategory {
    Desktop,
    Laptop,
    Server,
    NetworkSwitch,
    Router,
    Firewall,
    AccessPoint,
    Printer,
    Monitor,
    Phone,
    Tablet,
    Ups,
    Storage,
    Other(String),
}

/// Asset lifecycle status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetStatus {
    InStock,
    Deployed,
    Maintenance,
    Retired,
    Disposed,
    Lost,
    RMA,
}

/// Depreciation calculation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Depreciation {
    pub method: DepreciationMethod,
    pub useful_life_years: u32,
    pub salvage_value: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DepreciationMethod {
    StraightLine,
    DecliningBalance,
}

impl Depreciation {
    /// Calculate current book value using straight-line depreciation.
    pub fn current_value(&self, purchase_price: f64, purchase_date: NaiveDate) -> f64 {
        let today = Utc::now().date_naive();
        let days_owned = (today - purchase_date).num_days().max(0) as f64;
        let total_days = self.useful_life_years as f64 * 365.25;

        match self.method {
            DepreciationMethod::StraightLine => {
                let depreciation_per_day = (purchase_price - self.salvage_value) / total_days;
                let depreciated = depreciation_per_day * days_owned;
                (purchase_price - depreciated).max(self.salvage_value)
            }
            DepreciationMethod::DecliningBalance => {
                let rate = 2.0 / self.useful_life_years as f64;
                let years = days_owned / 365.25;
                let value = purchase_price * (1.0 - rate).powf(years);
                value.max(self.salvage_value)
            }
        }
    }
}

/// Warranty status check.
pub fn warranty_status(expires: Option<NaiveDate>) -> WarrantyStatus {
    match expires {
        None => WarrantyStatus::Unknown,
        Some(exp) => {
            let today = Utc::now().date_naive();
            let days_left = (exp - today).num_days();
            if days_left < 0 {
                WarrantyStatus::Expired
            } else if days_left <= 90 {
                WarrantyStatus::ExpiringSoon(days_left as u32)
            } else {
                WarrantyStatus::Active(days_left as u32)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WarrantyStatus {
    Active(u32),
    ExpiringSoon(u32),
    Expired,
    Unknown,
}

/// In-memory asset store.
#[derive(Debug, Default)]
pub struct AssetStore {
    assets: std::collections::HashMap<String, HardwareAsset>,
}

impl AssetStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, asset: HardwareAsset) {
        self.assets.insert(asset.id.clone(), asset);
    }

    pub fn get(&self, id: &str) -> Option<&HardwareAsset> {
        self.assets.get(id)
    }

    pub fn remove(&mut self, id: &str) -> Option<HardwareAsset> {
        self.assets.remove(id)
    }

    pub fn by_status(&self, status: &AssetStatus) -> Vec<&HardwareAsset> {
        self.assets
            .values()
            .filter(|a| &a.status == status)
            .collect()
    }

    pub fn by_vendor(&self, vendor: &str) -> Vec<&HardwareAsset> {
        self.assets
            .values()
            .filter(|a| a.vendor == vendor)
            .collect()
    }

    pub fn expiring_warranty(&self, within_days: i64) -> Vec<&HardwareAsset> {
        let cutoff = Utc::now().date_naive() + chrono::Duration::days(within_days);
        self.assets.values().filter(|a| {
            matches!(a.warranty_expires, Some(exp) if exp <= cutoff && exp >= Utc::now().date_naive())
        }).collect()
    }

    pub fn len(&self) -> usize {
        self.assets.len()
    }
    pub fn is_empty(&self) -> bool {
        self.assets.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_asset() -> HardwareAsset {
        let now = Utc::now();
        HardwareAsset {
            id: "a1".into(),
            name: "Dev Laptop".into(),
            asset_tag: Some("ASSET-001".into()),
            serial_number: Some("SN123".into()),
            model: "ThinkPad T14".into(),
            vendor: "Lenovo".into(),
            category: AssetCategory::Laptop,
            status: AssetStatus::Deployed,
            purchase_date: Some(NaiveDate::from_ymd_opt(2024, 1, 15).unwrap()),
            purchase_price: Some(1500.0),
            currency: Some("USD".into()),
            warranty_expires: Some(NaiveDate::from_ymd_opt(2027, 1, 15).unwrap()),
            invoice_ref: Some("INV-2024-001".into()),
            assigned_to: Some("alice@example.com".into()),
            location: Some("Office HQ".into()),
            notes: String::new(),
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn asset_serialization() {
        let a = sample_asset();
        let json = serde_json::to_string(&a).unwrap();
        let parsed: HardwareAsset = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "Dev Laptop");
        assert_eq!(parsed.vendor, "Lenovo");
    }

    #[test]
    fn depreciation_straight_line() {
        let dep = Depreciation {
            method: DepreciationMethod::StraightLine,
            useful_life_years: 5,
            salvage_value: 100.0,
        };
        let purchase = NaiveDate::from_ymd_opt(2020, 1, 1).unwrap();
        let value = dep.current_value(1500.0, purchase);
        // After 6+ years, should be at salvage value
        assert!((value - 100.0).abs() < 0.01);
    }

    #[test]
    fn warranty_active() {
        let future = Utc::now().date_naive() + chrono::Duration::days(365);
        assert!(matches!(
            warranty_status(Some(future)),
            WarrantyStatus::Active(_)
        ));
    }

    #[test]
    fn warranty_expired() {
        let past = NaiveDate::from_ymd_opt(2020, 1, 1).unwrap();
        assert_eq!(warranty_status(Some(past)), WarrantyStatus::Expired);
    }

    #[test]
    fn warranty_unknown() {
        assert_eq!(warranty_status(None), WarrantyStatus::Unknown);
    }

    #[test]
    fn asset_store_operations() {
        let mut store = AssetStore::new();
        store.insert(sample_asset());
        assert_eq!(store.len(), 1);
        assert!(store.get("a1").is_some());
        assert_eq!(store.by_status(&AssetStatus::Deployed).len(), 1);
        assert_eq!(store.by_vendor("Lenovo").len(), 1);
        store.remove("a1");
        assert!(store.is_empty());
    }
}
