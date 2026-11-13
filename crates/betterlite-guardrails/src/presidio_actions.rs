use std::time::{Duration, SystemTime};

use reqwest::Client;

pub mod anonymize;
pub mod blocking;
pub mod check;
pub mod error;
pub mod request_data;
pub mod tokens;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresidioEndpoints {
    pub analyzer_api_base: String,
    pub anonymizer_api_base: String,
}

#[derive(Debug, Clone)]
pub struct AnonymizerSettings {
    // ends with "/"; validate_environment normalizes it from PRESIDIO_ANONYMIZER_API_BASE
    pub anonymizer_api_base: String,
    pub timeout: Duration,
}

#[derive(Debug)]
pub struct PresidioAnonymizer {
    pub settings: AnonymizerSettings,
    pub client: Client,
    pub clock: fn() -> SystemTime,
}

pub fn validate_environment(
    analyzer_api_base: Option<&str>,
    anonymizer_api_base: Option<&str>,
    get_secret: fn(&str) -> Option<String>,
) -> Result<PresidioEndpoints, error::PresidioActionsError> {
    let analyzer = resolve_base(analyzer_api_base, "PRESIDIO_ANALYZER_API_BASE", get_secret)
        .ok_or(error::PresidioActionsError::MissingAnalyzerApiBase)?;
    let anonymizer = resolve_base(
        anonymizer_api_base,
        "PRESIDIO_ANONYMIZER_API_BASE",
        get_secret,
    )
    .ok_or(error::PresidioActionsError::MissingAnonymizerApiBase)?;
    Ok(PresidioEndpoints {
        analyzer_api_base: normalize_base(&analyzer),
        anonymizer_api_base: normalize_base(&anonymizer),
    })
}

fn resolve_base(
    argument: Option<&str>,
    variable: &str,
    get_secret: fn(&str) -> Option<String>,
) -> Option<String> {
    argument.filter(|value| !value.is_empty()).map_or_else(
        || get_secret(variable).filter(|value| !value.is_empty()),
        |value| Some(value.to_owned()),
    )
}

fn normalize_base(base: &str) -> String {
    let with_slash = if base.ends_with('/') {
        base.to_owned()
    } else {
        format!("{base}/")
    };
    if with_slash.starts_with("http://") || with_slash.starts_with("https://") {
        return with_slash;
    }
    // LiteLLM assumes a private-network address when the scheme is missing.
    format!("http://{with_slash}")
}

#[path = "presidio_actions/tests/anonymize_text.rs"]
mod anonymize_text;
#[path = "presidio_actions/tests/check_pii.rs"]
mod check_pii;
#[path = "presidio_actions/tests/check_pii_mock.rs"]
mod check_pii_mock;
#[path = "presidio_actions/tests/finalize_numbered.rs"]
mod finalize_numbered;
#[path = "presidio_actions/tests/finalize_simple.rs"]
mod finalize_simple;
#[path = "presidio_actions/tests/get_presidio_settings.rs"]
mod get_presidio_settings;
#[path = "presidio_actions/tests/post_anonymize.rs"]
mod post_anonymize;
#[path = "presidio_actions/tests/raise_blocked.rs"]
mod raise_blocked;
#[path = "presidio_actions/tests/shared.rs"]
mod shared;
#[path = "presidio_actions/tests/unmask.rs"]
mod unmask;
#[path = "presidio_actions/tests/unmask_truncation.rs"]
mod unmask_truncation;
#[path = "presidio_actions/tests/validate_environment.rs"]
mod validate_environment_tests;
