use std::collections::BTreeMap;

use regex::Regex;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::api::claude_plugin_policy::ClaudePluginPolicyError;

pub const GITHUB_REPO_MISSING: &str = "GitHub source must include 'repo' field (e.g., 'org/repo')";
pub const URL_FIELD_MISSING: &str =
    "URL source must include 'url' field (e.g., 'https://github.com/org/repo.git')";
pub const GIT_SUBDIR_URL_MISSING: &str =
    "git-subdir source must include 'url' field (e.g., 'https://github.com/org/repo.git')";
pub const GIT_SUBDIR_PATH_MISSING: &str =
    "git-subdir source must include 'path' field (e.g., 'plugins/plugin-name')";
pub const GIT_SUBDIR_PATH_INVALID: &str = "git-subdir 'path' must be a relative path of the form 'segment/segment' (alphanumeric, dots, hyphens, underscores only)";
pub const ARCHIVE_URL_INVALID: &str = "archive source must include an https 'url' field (e.g., 'https://bucket.s3.amazonaws.com/plugins/plugin-name.zip')";
pub const ARCHIVE_SHA256_INVALID: &str = "archive 'sha256' must be a 64-character hex digest";
pub const UNKNOWN_SOURCE: &str =
    "source.source must be 'github', 'url', 'git-subdir', or 'archive'";
pub const GIT_SUBDIR_PATH_PATTERN: &str =
    "^[a-zA-Z0-9][a-zA-Z0-9._-]*(/[a-zA-Z0-9][a-zA-Z0-9._-]*)*$";
pub const SHA256_PATTERN: &str = "^[0-9a-fA-F]{64}$";
pub const PATTERN_INVALID: &str = "a plugin source pattern cannot be compiled";

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct PluginSource {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, String>,
}

#[must_use]
pub fn is_https_url_with_host(text: &str) -> bool {
    // urlsplit reads a netloc only between "//" and the next "/", where WHATWG skips extra slashes.
    let cleaned: String = text
        .trim_start_matches(|character: char| character <= ' ')
        .chars()
        .filter(|character| !matches!(character, '\t' | '\n' | '\r'))
        .collect();
    let has_authority = cleaned
        .get(..8)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("https://"))
        && cleaned
            .get(8..)
            .is_some_and(|authority| !authority.starts_with('/'));
    has_authority && Url::parse(&cleaned).is_ok()
}

pub fn validate_plugin_source(source: &PluginSource) -> Result<(), ClaudePluginPolicyError> {
    match source.source.as_deref() {
        Some("github") => require_field(source.repo.is_some(), GITHUB_REPO_MISSING),
        Some("url") => require_field(source.url.is_some(), URL_FIELD_MISSING),
        Some("git-subdir" | "archive") => validate_variant_fields(source),
        _ => Err(ClaudePluginPolicyError::InvalidSource {
            message: UNKNOWN_SOURCE,
        }),
    }
}

fn require_field(present: bool, message: &'static str) -> Result<(), ClaudePluginPolicyError> {
    if present {
        return Ok(());
    }
    Err(ClaudePluginPolicyError::InvalidSource { message })
}

fn validate_variant_fields(source: &PluginSource) -> Result<(), ClaudePluginPolicyError> {
    if source.source.as_deref() == Some("git-subdir") {
        if source.url.as_deref().is_none_or(str::is_empty) {
            return Err(ClaudePluginPolicyError::InvalidSource {
                message: GIT_SUBDIR_URL_MISSING,
            });
        }
        let Some(path) = source.path.as_deref().filter(|path| !path.is_empty()) else {
            return Err(ClaudePluginPolicyError::InvalidSource {
                message: GIT_SUBDIR_PATH_MISSING,
            });
        };
        let pattern = Regex::new(GIT_SUBDIR_PATH_PATTERN).map_err(|source| {
            ClaudePluginPolicyError::Pattern {
                message: PATTERN_INVALID,
                source,
            }
        })?;
        if !pattern.is_match(path) {
            return Err(ClaudePluginPolicyError::InvalidSource {
                message: GIT_SUBDIR_PATH_INVALID,
            });
        }
        return Ok(());
    }
    if !is_https_url_with_host(source.url.as_deref().unwrap_or_default()) {
        return Err(ClaudePluginPolicyError::InvalidSource {
            message: ARCHIVE_URL_INVALID,
        });
    }
    let pattern =
        Regex::new(SHA256_PATTERN).map_err(|source| ClaudePluginPolicyError::Pattern {
            message: PATTERN_INVALID,
            source,
        })?;
    match source.sha256.as_deref() {
        Some(sha256) if !pattern.is_match(sha256) => Err(ClaudePluginPolicyError::InvalidSource {
            message: ARCHIVE_SHA256_INVALID,
        }),
        _ => Ok(()),
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PluginAuthor {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct PluginSpec {
    pub source: PluginSource,
    #[serde(default = "default_version")]
    pub version: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub author: Option<PluginAuthor>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub keywords: Option<Vec<String>>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub namespace: Option<String>,
}

fn default_version() -> Option<String> {
    Some("1.0.0".to_owned())
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct UpdatePluginSpec {
    pub source: PluginSource,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub author: Option<PluginAuthor>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub keywords: Option<Vec<String>>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub namespace: Option<String>,
}

impl From<UpdatePluginSpec> for PluginSpec {
    fn from(update: UpdatePluginSpec) -> Self {
        Self {
            source: update.source,
            version: update.version,
            description: update.description,
            author: update.author,
            homepage: update.homepage,
            keywords: update.keywords,
            category: update.category,
            domain: update.domain,
            namespace: update.namespace,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PluginManifest {
    pub name: String,
    pub source: PluginSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<PluginAuthor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keywords: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
}

#[must_use]
pub fn build_plugin_manifest(name: &str, spec: &PluginSpec) -> PluginManifest {
    PluginManifest {
        name: name.to_owned(),
        source: spec.source.clone(),
        version: spec.version.clone().filter(|version| !version.is_empty()),
        description: spec
            .description
            .clone()
            .filter(|description| !description.is_empty()),
        author: spec.author.clone(),
        homepage: spec
            .homepage
            .clone()
            .filter(|homepage| !homepage.is_empty()),
        keywords: spec
            .keywords
            .clone()
            .filter(|keywords| !keywords.is_empty()),
        category: spec
            .category
            .clone()
            .filter(|category| !category.is_empty()),
        domain: spec.domain.clone().filter(|domain| !domain.is_empty()),
        namespace: spec
            .namespace
            .clone()
            .filter(|namespace| !namespace.is_empty()),
    }
}
