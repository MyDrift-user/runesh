//! WinGet package manifest types (Microsoft.Rest source protocol).

use serde::{Deserialize, Serialize};

/// A package in the repository.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Package {
    pub package_identifier: String,
    pub versions: Vec<PackageVersion>,
}

/// A specific version of a package.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PackageVersion {
    pub package_version: String,
    pub default_locale: DefaultLocale,
    pub installers: Vec<Installer>,
    #[serde(default)]
    pub locales: Vec<serde_json::Value>,
}

/// Default locale metadata (required by winget).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DefaultLocale {
    pub package_locale: String,
    pub publisher: String,
    pub package_name: String,
    pub short_description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moniker: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

/// An installer for a package version.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Installer {
    pub architecture: Architecture,
    pub installer_type: InstallerType,
    pub installer_url: String,
    pub installer_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<Scope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installer_switches: Option<InstallerSwitches>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct InstallerSwitches {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub silent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub silent_with_progress: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Architecture {
    X86,
    X64,
    Arm64,
    Neutral,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InstallerType {
    Exe,
    Msi,
    Msix,
    Inno,
    Nullsoft,
    Wix,
    Burn,
    Zip,
    Portable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    User,
    Machine,
}

/// Search request from winget client.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SearchRequest {
    #[serde(default)]
    pub maximum_results: Option<u32>,
    #[serde(default)]
    pub fetch_all_manifests: Option<bool>,
    #[serde(default)]
    pub query: Option<SearchQuery>,
    #[serde(default)]
    pub filters: Vec<SearchFilter>,
    #[serde(default)]
    pub inclusions: Vec<SearchFilter>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SearchQuery {
    pub key_word: String,
    pub match_type: MatchType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SearchFilter {
    pub package_match_field: String,
    pub request_match: SearchQuery,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum MatchType {
    Exact,
    Partial,
    Substring,
    CaseInsensitive,
}

/// Search result entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SearchResultEntry {
    pub package_identifier: String,
    pub package_name: String,
    pub publisher: String,
    pub versions: Vec<SearchResultVersion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SearchResultVersion {
    pub package_version: String,
    #[serde(default)]
    pub package_family_names: Vec<String>,
    #[serde(default)]
    pub product_codes: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_serialization() {
        let pkg = Package {
            package_identifier: "MyOrg.MyApp".into(),
            versions: vec![PackageVersion {
                package_version: "1.0.0".into(),
                default_locale: DefaultLocale {
                    package_locale: "en-US".into(),
                    publisher: "MyOrg".into(),
                    package_name: "MyApp".into(),
                    short_description: "My application".into(),
                    publisher_url: Some("https://myorg.com".into()),
                    package_url: None,
                    license: Some("MIT".into()),
                    moniker: Some("myapp".into()),
                    tags: vec!["utility".into()],
                },
                installers: vec![Installer {
                    architecture: Architecture::X64,
                    installer_type: InstallerType::Msi,
                    installer_url: "https://myserver/downloads/myapp-1.0.0.msi".into(),
                    installer_sha256: "abc123def456".into(),
                    scope: Some(Scope::Machine),
                    installer_switches: Some(InstallerSwitches {
                        silent: Some("/quiet".into()),
                        silent_with_progress: Some("/passive".into()),
                    }),
                    product_code: None,
                }],
                locales: vec![],
            }],
        };

        let json = serde_json::to_string_pretty(&pkg).unwrap();
        assert!(json.contains("MyOrg.MyApp"));
        assert!(json.contains("PackageIdentifier"));

        let parsed: Package = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.package_identifier, "MyOrg.MyApp");
        assert_eq!(
            parsed.versions[0].installers[0].architecture,
            Architecture::X64
        );
    }

    #[test]
    fn search_request_deserialization() {
        let json = r#"{
            "MaximumResults": 10,
            "Query": {
                "KeyWord": "firefox",
                "MatchType": "Substring"
            }
        }"#;
        let req: SearchRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.maximum_results, Some(10));
        assert_eq!(req.query.unwrap().key_word, "firefox");
    }
}
