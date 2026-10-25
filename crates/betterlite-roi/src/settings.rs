use secrecy::SecretString;
use serde::{Deserialize, Serialize, de::Deserializer};
use std::collections::BTreeMap;
use thiserror::Error;
use url::Url;

pub const DEFAULT_PROMPT: &str = "Estimate how many hours it would take an engineer to complete the work in this pull request without AI assistance. Explain your estimate briefly.";

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error("Choose manual updates (0), or an interval of at least 5 minutes.")]
    Interval,
    #[error("Use an HTTPS GitHub API URL without credentials, query, or fragment.")]
    GitHubUrl,
    #[error("failed to parse GitHub API URL: {source}")]
    GitHubUrlParse {
        #[source]
        source: url::ParseError,
    },
    #[error("Repositories must use owner/repo format.")]
    Repositories,
    #[error("The estimator prompt must contain between 1 and 20,000 characters.")]
    Prompt,
    #[error("Each identity needs a GitHub username and a valid gateway email.")]
    Identity,
    #[error("Enter a valid GitHub username.")]
    Login,
    #[error("Backfill days must be between 1 and 3650.")]
    BackfillDays,
}

pub fn normalize_login(value: &str) -> Result<String, SettingsError> {
    let login = value.trim().to_lowercase();
    if login.is_empty()
        || login
            .chars()
            .any(|character| !(character.is_ascii_alphanumeric() || "_[]-".contains(character)))
    {
        return Err(SettingsError::Login);
    }
    Ok(login)
}

pub fn normalize_email(value: Option<&str>) -> String {
    let normalized = value.unwrap_or_default().trim().to_lowercase();
    let Some((local, domain)) = normalized.rsplit_once('@') else {
        return String::new();
    };
    let valid = !local.is_empty()
        && !domain.is_empty()
        && domain.contains('.')
        && !normalized.chars().any(char::is_whitespace)
        && normalized.matches('@').count() == 1
        && !domain.ends_with("noreply.github.com");
    if valid { normalized } else { String::new() }
}

pub fn validate_update_interval(value: f64) -> Result<f64, SettingsError> {
    if !value.is_finite() || !(value == 0.0_f64 || (5.0_f64..=43_200.0_f64).contains(&value)) {
        return Err(SettingsError::Interval);
    }
    Ok(value)
}

pub fn normalize_github_api_url(value: &str) -> Result<String, SettingsError> {
    let normalized = value.trim().trim_end_matches('/');
    if normalized.is_empty() {
        return Err(SettingsError::GitHubUrl);
    }
    let parsed =
        Url::parse(normalized).map_err(|source| SettingsError::GitHubUrlParse { source })?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(SettingsError::GitHubUrl);
    }
    Ok(normalized.to_owned())
}

pub fn validate_repositories(values: &[String]) -> Result<Vec<String>, SettingsError> {
    let mut normalized = Vec::new();
    for value in values {
        let repository = value
            .trim()
            .trim_end_matches('/')
            .trim_end_matches(".git")
            .to_owned();
        if !normalized.contains(&repository) {
            normalized.push(repository);
        }
    }
    if normalized.iter().any(|repository| {
        let mut parts = repository.split('/');
        let Some(owner) = parts.next() else {
            return true;
        };
        let Some(name) = parts.next() else {
            return true;
        };
        parts.next().is_some()
            || owner.is_empty()
            || name.is_empty()
            || owner == "."
            || owner == ".."
            || name == "."
            || name == ".."
            || owner
                .chars()
                .any(|character| !(character.is_ascii_alphanumeric() || "_.-".contains(character)))
            || name
                .chars()
                .any(|character| !(character.is_ascii_alphanumeric() || "_.-".contains(character)))
    }) {
        return Err(SettingsError::Repositories);
    }
    Ok(normalized)
}

pub fn validate_estimator_prompt(value: &str) -> Result<String, SettingsError> {
    let normalized = value.trim();
    if normalized.is_empty() || normalized.chars().count() > 20_000 {
        return Err(SettingsError::Prompt);
    }
    Ok(normalized.to_owned())
}

pub fn normalize_identity_map(
    values: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, SettingsError> {
    let mut normalized = BTreeMap::new();
    for (login, email) in values {
        let normalized_login = normalize_login(login)?;
        let normalized_email = normalize_email(Some(email));
        if normalized_email.is_empty() {
            return Err(SettingsError::Identity);
        }
        normalized.insert(normalized_login, normalized_email);
    }
    Ok(normalized)
}

#[derive(Debug, Clone, Deserialize)]
pub struct ROISettings {
    #[serde(default = "default_github_url")]
    pub github_api_url: String,
    #[serde(default, deserialize_with = "deserialize_secret")]
    pub github_token: SecretString,
    #[serde(default, deserialize_with = "deserialize_secret")]
    pub estimator_key: SecretString,
    #[serde(default)]
    pub repos: Vec<String>,
    #[serde(default)]
    pub estimator_model: String,
    #[serde(default = "default_prompt_value")]
    pub estimator_prompt: String,
    #[serde(default = "default_backfill_days")]
    pub backfill_days: u32,
    #[serde(default = "default_interval")]
    pub update_interval_minutes: f64,
    #[serde(default)]
    pub identity_map: BTreeMap<String, String>,
}

impl Default for ROISettings {
    fn default() -> Self {
        Self {
            github_api_url: default_github_url(),
            github_token: SecretString::from(String::new()),
            estimator_key: SecretString::from(String::new()),
            repos: Vec::new(),
            estimator_model: String::new(),
            estimator_prompt: default_prompt_value(),
            backfill_days: default_backfill_days(),
            update_interval_minutes: default_interval(),
            identity_map: BTreeMap::new(),
        }
    }
}

impl ROISettings {
    pub fn validate(mut self) -> Result<Self, SettingsError> {
        self.github_api_url = normalize_github_api_url(&self.github_api_url)?;
        self.repos = validate_repositories(&self.repos)?;
        self.estimator_prompt = validate_estimator_prompt(&self.estimator_prompt)?;
        self.update_interval_minutes = validate_update_interval(self.update_interval_minutes)?;
        if !(1..=3650).contains(&self.backfill_days) {
            return Err(SettingsError::BackfillDays);
        }
        self.identity_map = normalize_identity_map(&self.identity_map)?;
        Ok(self)
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ROISettingsUpdate {
    pub github_api_url: Option<String>,
    #[serde(deserialize_with = "deserialize_secret_option", default)]
    pub github_token: Option<SecretString>,
    #[serde(deserialize_with = "deserialize_secret_option", default)]
    pub estimator_key: Option<SecretString>,
    pub repos: Option<Vec<String>>,
    pub estimator_model: Option<String>,
    pub estimator_prompt: Option<String>,
    pub backfill_days: Option<u32>,
    pub update_interval_minutes: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ROISettingsResponse {
    pub github_api_url: String,
    pub repos: Vec<String>,
    pub estimator_model: String,
    pub estimator_prompt: String,
    pub backfill_days: u32,
    pub update_interval_minutes: f64,
    pub has_estimator_key: SettingFlag,
    pub identity_map: BTreeMap<String, String>,
    pub has_github_token: SettingFlag,
    pub default_prompt: String,
    pub available_models: Vec<String>,
    pub ready: SettingFlag,
}

pub type SettingFlag = bool;

fn default_github_url() -> String {
    "https://api.github.com".to_owned()
}
fn default_prompt_value() -> String {
    DEFAULT_PROMPT.to_owned()
}
const fn default_backfill_days() -> u32 {
    7
}
const fn default_interval() -> f64 {
    1440.0
}

fn deserialize_secret<'de, D>(deserializer: D) -> Result<SecretString, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    Ok(SecretString::from(value))
}

fn deserialize_secret_option<'de, D>(deserializer: D) -> Result<Option<SecretString>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    Ok(value.map(SecretString::from))
}

#[cfg(test)]
mod tests;
