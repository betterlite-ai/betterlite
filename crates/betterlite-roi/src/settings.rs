use indexmap::IndexMap;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const DEFAULT_PROMPT: &str = "Estimate how many hours it would take an engineer to complete the work in this pull request without AI assistance. Explain your estimate briefly.";
// what Python's str.strip() and the re module's \s treat as whitespace
const PYTHON_SPACE: [char; 29] = [
    '\t', '\n', '\u{b}', '\u{c}', '\r', '\u{1c}', '\u{1d}', '\u{1e}', '\u{1f}', ' ', '\u{85}',
    '\u{a0}', '\u{1680}', '\u{2000}', '\u{2001}', '\u{2002}', '\u{2003}', '\u{2004}', '\u{2005}',
    '\u{2006}', '\u{2007}', '\u{2008}', '\u{2009}', '\u{200a}', '\u{2028}', '\u{2029}', '\u{202f}',
    '\u{205f}', '\u{3000}',
];

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SettingsError {
    #[error("Input should be a finite number")]
    NotFinite { field: &'static str },
    #[error("Input should be greater than or equal to {limit}")]
    TooSmall { field: &'static str, limit: u32 },
    #[error("Input should be less than or equal to {limit}")]
    TooLarge { field: &'static str, limit: u32 },
    #[error("Choose manual updates (0), or an interval of at least 5 minutes.")]
    Interval,
    #[error("A GitHub API URL is required.")]
    MissingGitHubUrl,
    #[error("Use an HTTPS GitHub API URL without credentials, query, or fragment.")]
    GitHubUrl,
    #[error("Invalid IPv6 URL")]
    Ipv6Url,
    #[error("IPvFuture address is invalid")]
    IpvFuture,
    #[error("An IPv4 address cannot be in brackets")]
    BracketedIpv4,
    #[error("'{host}' does not appear to be an IPv4 or IPv6 address")]
    BracketedHost { host: String },
    #[error("netloc '{netloc}' contains invalid characters under NFKC normalization")]
    NetlocNormalization { netloc: String },
    #[error("Repositories must use owner/repo format.")]
    Repositories,
    #[error("The estimator prompt must contain between 1 and 20,000 characters.")]
    Prompt,
    #[error("Enter a valid GitHub username.")]
    Login,
    #[error("Each identity needs a GitHub username and a valid gateway email.")]
    Identity,
}
pub fn normalize_login(value: &str) -> Result<String, SettingsError> {
    let login = caseless::default_case_fold_str(value.trim_matches(PYTHON_SPACE));
    if login.is_empty()
        || !login
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_[]-".contains(character))
    {
        return Err(SettingsError::Login);
    }
    Ok(login)
}
#[must_use]
pub fn normalize_email(value: &str) -> String {
    let normalized = caseless::default_case_fold_str(value.trim_matches(PYTHON_SPACE));
    let Some((local, domain)) = normalized.split_once('@') else {
        return String::new();
    };
    let mut inner = domain.chars();
    inner.next();
    inner.next_back();
    if local.is_empty()
        || domain.contains('@')
        || normalized.contains(PYTHON_SPACE)
        || !inner.as_str().contains('.')
        || normalized.ends_with("noreply.github.com")
    {
        return String::new();
    }
    normalized
}
const fn check_backfill_days(value: i64) -> Result<(), SettingsError> {
    if value < 1 {
        return Err(SettingsError::TooSmall {
            field: "backfill_days",
            limit: 1,
        });
    }
    if value > 3650 {
        return Err(SettingsError::TooLarge {
            field: "backfill_days",
            limit: 3650,
        });
    }
    Ok(())
}
fn check_interval_bounds(value: f64) -> Result<(), SettingsError> {
    let field = "update_interval_minutes";
    if !value.is_finite() {
        return Err(SettingsError::NotFinite { field });
    }
    if value < 0.0_f64 {
        return Err(SettingsError::TooSmall { field, limit: 0 });
    }
    if value > 43_200.0_f64 {
        return Err(SettingsError::TooLarge {
            field,
            limit: 43_200,
        });
    }
    Ok(())
}

pub fn validate_update_interval(value: f64) -> Result<f64, SettingsError> {
    check_interval_bounds(value)?;
    if 0.0_f64 < value && value < 5.0_f64 {
        return Err(SettingsError::Interval);
    }
    Ok(value)
}

pub fn validate_repositories(values: &[String]) -> Result<Vec<String>, SettingsError> {
    let mut normalized: Vec<String> = Vec::new();
    for value in values {
        let stripped = value.trim_matches(PYTHON_SPACE).trim_end_matches('/');
        let repository = stripped.strip_suffix(".git").unwrap_or(stripped);
        if !normalized.iter().any(|existing| existing == repository) {
            normalized.push(repository.to_owned());
        }
    }
    let valid_part = |part: &str| {
        !part.is_empty()
            && part != "."
            && part != ".."
            && part
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "_.-".contains(character))
    };
    let valid = normalized.iter().all(|repository| {
        repository
            .split_once('/')
            .is_some_and(|(owner, name)| valid_part(owner) && valid_part(name))
    });
    if !valid {
        return Err(SettingsError::Repositories);
    }
    Ok(normalized)
}

pub fn validate_estimator_prompt(value: &str) -> Result<String, SettingsError> {
    let normalized = value.trim_matches(PYTHON_SPACE);
    if normalized.is_empty() || normalized.chars().count() > 20_000 {
        return Err(SettingsError::Prompt);
    }
    Ok(normalized.to_owned())
}

pub fn normalize_identity_map(
    values: &IndexMap<String, String>,
) -> Result<IndexMap<String, String>, SettingsError> {
    let mut normalized = IndexMap::new();
    for (login, address) in values {
        let email = normalize_email(address);
        if !email.is_empty() {
            normalized.insert(normalize_login(login)?, email);
        }
    }
    if normalized.len() != values.len() {
        return Err(SettingsError::Identity);
    }
    Ok(normalized)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ROISettings {
    pub github_api_url: String,
    pub github_token: SecretString,
    pub estimator_key: SecretString,
    pub repos: Vec<String>,
    pub estimator_model: String,
    pub estimator_prompt: String,
    pub backfill_days: i64,
    pub update_interval_minutes: f64,
    pub identity_map: IndexMap<String, String>,
}

impl Default for ROISettings {
    fn default() -> Self {
        Self {
            github_api_url: "https://api.github.com".to_owned(),
            github_token: SecretString::default(),
            estimator_key: SecretString::default(),
            repos: Vec::new(),
            estimator_model: String::new(),
            estimator_prompt: DEFAULT_PROMPT.to_owned(),
            backfill_days: 7,
            update_interval_minutes: 1440.0_f64,
            identity_map: IndexMap::new(),
        }
    }
}

impl ROISettings {
    pub fn validate(mut self) -> Result<Self, SettingsError> {
        self.github_api_url = github_url::normalize_github_api_url(&self.github_api_url)?;
        self.repos = validate_repositories(&self.repos)?;
        self.estimator_prompt = validate_estimator_prompt(&self.estimator_prompt)?;
        check_backfill_days(self.backfill_days)?;
        self.update_interval_minutes = validate_update_interval(self.update_interval_minutes)?;
        self.identity_map = normalize_identity_map(&self.identity_map)?;
        Ok(self)
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ROISettingsUpdate {
    pub github_api_url: Option<String>,
    pub github_token: Option<SecretString>,
    pub estimator_key: Option<SecretString>,
    pub repos: Option<Vec<String>>,
    pub estimator_model: Option<String>,
    pub estimator_prompt: Option<String>,
    pub backfill_days: Option<i64>,
    pub update_interval_minutes: Option<f64>,
}

impl ROISettingsUpdate {
    pub fn validate(self) -> Result<Self, SettingsError> {
        if let Some(days) = self.backfill_days {
            check_backfill_days(days)?;
        }
        if let Some(minutes) = self.update_interval_minutes {
            check_interval_bounds(minutes)?;
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "LiteLLM's settings response carries three independent flags on the wire"
)]
pub struct ROISettingsResponse {
    pub github_api_url: String,
    pub repos: Vec<String>,
    pub estimator_model: String,
    pub estimator_prompt: String,
    pub backfill_days: i64,
    pub update_interval_minutes: f64,
    pub has_estimator_key: bool,
    pub identity_map: IndexMap<String, String>,
    pub has_github_token: bool,
    pub default_prompt: String,
    pub available_models: Vec<String>,
    pub ready: bool,
}

pub mod github_url;
mod tests;
