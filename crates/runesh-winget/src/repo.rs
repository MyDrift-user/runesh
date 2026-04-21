//! WinGet package repository store and search.

use std::collections::HashMap;

use crate::manifest::{
    MatchType, Package, PackageVersion, SearchFilter, SearchRequest, SearchResultEntry,
    SearchResultVersion,
};

/// In-memory WinGet package repository.
#[derive(Debug, Default)]
pub struct WingetRepo {
    /// Packages indexed by PackageIdentifier.
    packages: HashMap<String, Package>,
    /// Source identifier for the /information endpoint.
    pub source_identifier: String,
}

impl WingetRepo {
    pub fn new(source_identifier: &str) -> Self {
        Self {
            packages: HashMap::new(),
            source_identifier: source_identifier.to_string(),
        }
    }

    /// Add or update a package.
    pub fn upsert_package(&mut self, package: Package) {
        self.packages
            .insert(package.package_identifier.clone(), package);
    }

    /// Add a version to an existing package (or create the package).
    pub fn add_version(&mut self, package_id: &str, version: PackageVersion) {
        let pkg = self
            .packages
            .entry(package_id.to_string())
            .or_insert_with(|| Package {
                package_identifier: package_id.to_string(),
                versions: vec![],
            });

        // Replace existing version or add new
        if let Some(existing) = pkg
            .versions
            .iter_mut()
            .find(|v| v.package_version == version.package_version)
        {
            *existing = version;
        } else {
            pkg.versions.push(version);
        }
    }

    /// Remove a package entirely.
    pub fn remove_package(&mut self, package_id: &str) -> Option<Package> {
        self.packages.remove(package_id)
    }

    /// Remove a specific version of a package.
    pub fn remove_version(&mut self, package_id: &str, version: &str) -> bool {
        let removed = if let Some(pkg) = self.packages.get_mut(package_id) {
            let before = pkg.versions.len();
            pkg.versions.retain(|v| v.package_version != version);
            pkg.versions.len() < before
        } else {
            return false;
        };
        // Clean up empty packages
        if self
            .packages
            .get(package_id)
            .map(|p| p.versions.is_empty())
            .unwrap_or(false)
        {
            self.packages.remove(package_id);
        }
        removed
    }

    /// Get a package by identifier.
    pub fn get_package(&self, package_id: &str) -> Option<&Package> {
        self.packages.get(package_id)
    }

    /// Search packages using the winget search protocol.
    pub fn search(&self, request: &SearchRequest) -> Vec<SearchResultEntry> {
        let max = request.maximum_results.unwrap_or(50) as usize;

        let mut results: Vec<SearchResultEntry> = self
            .packages
            .values()
            .filter(|pkg| {
                // Apply query (global keyword search)
                if let Some(query) = &request.query
                    && !matches_keyword(pkg, &query.key_word, &query.match_type)
                {
                    return false;
                }

                // Apply filters (all must match)
                for filter in &request.filters {
                    if !matches_filter(pkg, filter) {
                        return false;
                    }
                }

                true
            })
            .map(|pkg| {
                let latest = pkg.versions.last();
                SearchResultEntry {
                    package_identifier: pkg.package_identifier.clone(),
                    package_name: latest
                        .map(|v| v.default_locale.package_name.clone())
                        .unwrap_or_default(),
                    publisher: latest
                        .map(|v| v.default_locale.publisher.clone())
                        .unwrap_or_default(),
                    versions: pkg
                        .versions
                        .iter()
                        .map(|v| SearchResultVersion {
                            package_version: v.package_version.clone(),
                            package_family_names: vec![],
                            product_codes: v
                                .installers
                                .iter()
                                .filter_map(|i| i.product_code.clone())
                                .collect(),
                        })
                        .collect(),
                }
            })
            .collect();

        results.truncate(max);
        results
    }

    /// Total packages in the repository.
    pub fn package_count(&self) -> usize {
        self.packages.len()
    }

    /// Total versions across all packages.
    pub fn version_count(&self) -> usize {
        self.packages.values().map(|p| p.versions.len()).sum()
    }

    /// Export the full repository as JSON.
    pub fn export(&self) -> Result<String, serde_json::Error> {
        let pkgs: Vec<&Package> = self.packages.values().collect();
        serde_json::to_string_pretty(&pkgs)
    }

    /// Import packages from JSON.
    pub fn import(&mut self, json: &str) -> Result<usize, serde_json::Error> {
        let pkgs: Vec<Package> = serde_json::from_str(json)?;
        let count = pkgs.len();
        for pkg in pkgs {
            self.upsert_package(pkg);
        }
        Ok(count)
    }
}

fn matches_keyword(pkg: &Package, keyword: &str, match_type: &MatchType) -> bool {
    let fields = [
        &pkg.package_identifier,
        &pkg.versions
            .last()
            .map(|v| v.default_locale.package_name.clone())
            .unwrap_or_default(),
        &pkg.versions
            .last()
            .map(|v| v.default_locale.publisher.clone())
            .unwrap_or_default(),
    ];

    let kw_lower = keyword.to_lowercase();

    fields.iter().any(|f| match match_type {
        MatchType::Exact => f.as_str() == keyword,
        MatchType::CaseInsensitive => f.to_lowercase() == kw_lower,
        MatchType::Substring | MatchType::Partial => f.to_lowercase().contains(&kw_lower),
    })
}

fn matches_filter(pkg: &Package, filter: &SearchFilter) -> bool {
    let latest = pkg.versions.last();
    let field_value = match filter.package_match_field.as_str() {
        "PackageIdentifier" => pkg.package_identifier.clone(),
        "PackageName" => latest
            .map(|v| v.default_locale.package_name.clone())
            .unwrap_or_default(),
        "Publisher" => latest
            .map(|v| v.default_locale.publisher.clone())
            .unwrap_or_default(),
        "Moniker" => latest
            .and_then(|v| v.default_locale.moniker.clone())
            .unwrap_or_default(),
        "Tag" => {
            let kw = &filter.request_match.key_word;
            return latest
                .map(|v| v.default_locale.tags.iter().any(|t| t == kw))
                .unwrap_or(false);
        }
        _ => return true, // unknown field, don't filter
    };

    let kw = &filter.request_match.key_word;
    match filter.request_match.match_type {
        MatchType::Exact => field_value == *kw,
        MatchType::CaseInsensitive => field_value.to_lowercase() == kw.to_lowercase(),
        MatchType::Substring | MatchType::Partial => {
            field_value.to_lowercase().contains(&kw.to_lowercase())
        }
    }
}

/// Test helpers (available within the crate for integration tests).
#[doc(hidden)]
pub mod tests_helper {

    use crate::manifest::*;

    pub fn sample_package(id: &str, name: &str, version: &str) -> Package {
        Package {
            package_identifier: id.into(),
            versions: vec![PackageVersion {
                package_version: version.into(),
                default_locale: DefaultLocale {
                    package_locale: "en-US".into(),
                    publisher: "TestOrg".into(),
                    package_name: name.into(),
                    short_description: format!("{name} application"),
                    publisher_url: None,
                    package_url: None,
                    license: None,
                    moniker: None,
                    tags: vec!["test".into()],
                },
                installers: vec![Installer {
                    architecture: Architecture::X64,
                    installer_type: InstallerType::Msi,
                    installer_url: format!("https://example.com/{id}-{version}.msi"),
                    installer_sha256: "abc123".into(),
                    scope: Some(Scope::Machine),
                    installer_switches: None,
                    product_code: None,
                }],
                locales: vec![],
            }],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::*;
    use tests_helper::sample_package;

    #[test]
    fn add_and_get_package() {
        let mut repo = WingetRepo::new("TestRepo");
        repo.upsert_package(sample_package("Test.App", "TestApp", "1.0.0"));
        assert_eq!(repo.package_count(), 1);
        assert!(repo.get_package("Test.App").is_some());
    }

    #[test]
    fn add_version_to_existing() {
        let mut repo = WingetRepo::new("TestRepo");
        repo.upsert_package(sample_package("Test.App", "TestApp", "1.0.0"));

        let v2 = PackageVersion {
            package_version: "2.0.0".into(),
            default_locale: DefaultLocale {
                package_locale: "en-US".into(),
                publisher: "TestOrg".into(),
                package_name: "TestApp".into(),
                short_description: "Updated".into(),
                publisher_url: None,
                package_url: None,
                license: None,
                moniker: None,
                tags: vec![],
            },
            installers: vec![],
            locales: vec![],
        };
        repo.add_version("Test.App", v2);
        assert_eq!(repo.get_package("Test.App").unwrap().versions.len(), 2);
    }

    #[test]
    fn search_by_keyword() {
        let mut repo = WingetRepo::new("TestRepo");
        repo.upsert_package(sample_package("Mozilla.Firefox", "Firefox", "125.0"));
        repo.upsert_package(sample_package("Google.Chrome", "Chrome", "124.0"));

        let req = SearchRequest {
            query: Some(SearchQuery {
                key_word: "firefox".into(),
                match_type: MatchType::Substring,
            }),
            ..Default::default()
        };
        let results = repo.search(&req);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].package_identifier, "Mozilla.Firefox");
    }

    #[test]
    fn search_by_exact_id() {
        let mut repo = WingetRepo::new("TestRepo");
        repo.upsert_package(sample_package("Test.App", "TestApp", "1.0.0"));
        repo.upsert_package(sample_package("Test.App2", "TestApp2", "1.0.0"));

        let req = SearchRequest {
            filters: vec![SearchFilter {
                package_match_field: "PackageIdentifier".into(),
                request_match: SearchQuery {
                    key_word: "Test.App".into(),
                    match_type: MatchType::Exact,
                },
            }],
            ..Default::default()
        };
        let results = repo.search(&req);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn remove_package() {
        let mut repo = WingetRepo::new("TestRepo");
        repo.upsert_package(sample_package("Test.App", "TestApp", "1.0.0"));
        assert!(repo.remove_package("Test.App").is_some());
        assert_eq!(repo.package_count(), 0);
    }

    #[test]
    fn export_import() {
        let mut repo = WingetRepo::new("TestRepo");
        repo.upsert_package(sample_package("A.B", "AB", "1.0"));
        repo.upsert_package(sample_package("C.D", "CD", "2.0"));

        let json = repo.export().unwrap();
        let mut repo2 = WingetRepo::new("TestRepo2");
        repo2.import(&json).unwrap();
        assert_eq!(repo2.package_count(), 2);
    }
}
