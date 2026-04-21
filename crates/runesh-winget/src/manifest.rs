//! WinGet package manifest types (Microsoft.Rest source protocol).

use serde::{Deserialize, Serialize};

/// Errors produced while validating manifest content.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("installer_sha256 must be 64 hex characters")]
    InvalidHash,
    #[error("installer_url invalid: {0}")]
    InvalidUrl(String),
    #[error("http scheme is not allowed unless the caller opts in")]
    HttpNotAllowed,
}

/// Policy for validating installer manifests.
#[derive(Debug, Clone, Copy, Default)]
pub struct ManifestPolicy {
    /// When `true`, `http://` URLs are accepted. `https` is always accepted.
    pub allow_http: bool,
}

/// Validate that a string matches `^[a-fA-F0-9]{64}$`.
pub fn validate_sha256(hash: &str) -> Result<(), ManifestError> {
    if hash.len() != 64 {
        return Err(ManifestError::InvalidHash);
    }
    if !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ManifestError::InvalidHash);
    }
    Ok(())
}

/// Validate that a URL parses and uses an allowed scheme.
pub fn validate_installer_url(url: &str, policy: ManifestPolicy) -> Result<(), ManifestError> {
    let parsed = url::Url::parse(url).map_err(|e| ManifestError::InvalidUrl(e.to_string()))?;
    match parsed.scheme() {
        "https" => Ok(()),
        "http" if policy.allow_http => Ok(()),
        "http" => Err(ManifestError::HttpNotAllowed),
        other => Err(ManifestError::InvalidUrl(format!(
            "scheme {other} not allowed"
        ))),
    }
}

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

impl Installer {
    /// Validate the installer fields against `policy`.
    pub fn validate(&self, policy: ManifestPolicy) -> Result<(), ManifestError> {
        validate_sha256(&self.installer_sha256)?;
        validate_installer_url(&self.installer_url, policy)?;
        Ok(())
    }
}

impl Package {
    /// Validate every installer under every version of the package.
    pub fn validate(&self, policy: ManifestPolicy) -> Result<(), ManifestError> {
        for v in &self.versions {
            for i in &v.installers {
                i.validate(policy)?;
            }
        }
        Ok(())
    }
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
    fn sha256_validator_accepts_64_hex() {
        let ok = "a".repeat(64);
        assert!(validate_sha256(&ok).is_ok());
        let mixed = format!("{}{}", "A".repeat(32), "f".repeat(32));
        assert!(validate_sha256(&mixed).is_ok());
    }

    #[test]
    fn sha256_validator_rejects_other_inputs() {
        assert!(validate_sha256("").is_err());
        assert!(validate_sha256(&"a".repeat(63)).is_err());
        assert!(validate_sha256(&"a".repeat(65)).is_err());
        assert!(validate_sha256(&format!("{}g", "a".repeat(63))).is_err());
        assert!(validate_sha256("not a hash").is_err());
    }

    #[test]
    fn installer_url_scheme_policy() {
        let policy = ManifestPolicy { allow_http: false };
        assert!(validate_installer_url("https://example.com/x.msi", policy).is_ok());
        assert!(matches!(
            validate_installer_url("http://example.com/x.msi", policy),
            Err(ManifestError::HttpNotAllowed)
        ));
        let allow = ManifestPolicy { allow_http: true };
        assert!(validate_installer_url("http://example.com/x.msi", allow).is_ok());
        assert!(validate_installer_url("ftp://example.com/x.msi", allow).is_err());
        assert!(validate_installer_url("not a url", allow).is_err());
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
