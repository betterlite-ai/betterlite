use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use std::num::ParseFloatError;
use thiserror::Error;

#[path = "resource_sdk_settings/tests.rs"]
mod tests;

#[derive(Clone, Debug)]
pub struct ResourceSdkSettings {
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub api_key: Option<SecretString>,
    pub openai_key: Option<SecretString>,
    pub azure_key: Option<SecretString>,
    pub azure_ad_token: Option<SecretString>,
    pub organization: Option<String>,
    pub vertex_project: Option<String>,
    pub vertex_location: Option<String>,
    pub openai_base_url: Option<String>,
    pub openai_api_base: Option<String>,
    pub openai_api_key: Option<SecretString>,
    pub openai_organization: Option<String>,
    pub azure_api_base: Option<String>,
    pub azure_api_version: Option<String>,
    pub azure_openai_api_key: Option<SecretString>,
    pub azure_api_key: Option<SecretString>,
    pub vertexai_project: Option<String>,
    pub vertexai_location: Option<String>,
    pub vertexai_credentials: Option<SecretString>,
}

#[derive(Debug, Default)]
pub struct ResourceSdkRequestOptions {
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub api_key: Option<SecretString>,
    pub organization: Option<String>,
    pub vertex_project: Option<String>,
    pub vertex_location: Option<String>,
    pub vertex_credentials: Option<SecretString>,
}

impl ResourceSdkSettings {
    #[must_use]
    pub fn openai_base(&self, request: &ResourceSdkRequestOptions) -> String {
        first_truthy([
            request.api_base.as_deref(),
            self.api_base.as_deref(),
            self.openai_base_url.as_deref(),
            self.openai_api_base.as_deref(),
            Some("https://api.openai.com/v1"),
        ])
        .to_owned()
    }

    #[must_use]
    pub fn openai_api_key<'settings>(
        &'settings self,
        request: &'settings ResourceSdkRequestOptions,
    ) -> Option<&'settings SecretString> {
        first_secret([
            request.api_key.as_ref(),
            self.api_key.as_ref(),
            self.openai_key.as_ref(),
            self.openai_api_key.as_ref(),
        ])
    }

    #[must_use]
    pub fn azure_api_key<'settings>(
        &'settings self,
        request: &'settings ResourceSdkRequestOptions,
    ) -> Option<&'settings SecretString> {
        let candidates = [
            request.api_key.as_ref(),
            self.api_key.as_ref(),
            self.azure_key.as_ref(),
            self.azure_openai_api_key.as_ref(),
            self.azure_api_key.as_ref(),
        ];
        first_secret(candidates)
    }

    #[must_use]
    pub fn azure_base<'settings>(
        &'settings self,
        request: &'settings ResourceSdkRequestOptions,
    ) -> Option<&'settings str> {
        let value = first_truthy([
            request.api_base.as_deref(),
            self.api_base.as_deref(),
            self.azure_api_base.as_deref(),
        ]);
        (!value.is_empty()).then_some(value)
    }

    #[must_use]
    pub fn azure_version<'settings>(
        &'settings self,
        request: &'settings ResourceSdkRequestOptions,
    ) -> Option<&'settings str> {
        let value = first_truthy([
            request.api_version.as_deref(),
            self.api_version.as_deref(),
            self.azure_api_version.as_deref(),
        ]);
        (!value.is_empty()).then_some(value)
    }

    #[must_use]
    pub fn openai_organization<'settings>(
        &'settings self,
        request: &'settings ResourceSdkRequestOptions,
    ) -> Option<&'settings str> {
        let value = first_truthy([
            request.organization.as_deref(),
            self.organization.as_deref(),
            self.openai_organization.as_deref(),
        ]);
        (!value.is_empty()).then_some(value)
    }

    #[must_use]
    pub fn openai_api_version<'settings>(
        &'settings self,
        request: &'settings ResourceSdkRequestOptions,
    ) -> Option<&'settings str> {
        request
            .api_version
            .as_deref()
            .filter(|value| !value.is_empty())
    }

    #[must_use]
    pub fn azure_organization<'settings>(
        &'settings self,
        request: &'settings ResourceSdkRequestOptions,
    ) -> Option<&'settings str> {
        request
            .organization
            .as_deref()
            .filter(|value| !value.is_empty())
    }

    #[must_use]
    pub fn vertex_base<'settings>(
        &'settings self,
        request: &'settings ResourceSdkRequestOptions,
    ) -> &'settings str {
        request.api_base.as_deref().unwrap_or("")
    }

    #[must_use]
    pub fn vertex_project<'settings>(
        &'settings self,
        request: &'settings ResourceSdkRequestOptions,
    ) -> Option<&'settings str> {
        let value = first_truthy([
            request.vertex_project.as_deref(),
            self.vertex_project.as_deref(),
            self.vertexai_project.as_deref(),
        ]);
        (!value.is_empty()).then_some(value)
    }

    #[must_use]
    pub fn vertex_location<'settings>(
        &'settings self,
        request: &'settings ResourceSdkRequestOptions,
    ) -> Option<&'settings str> {
        let value = first_truthy([
            request.vertex_location.as_deref(),
            self.vertex_location.as_deref(),
            self.vertexai_location.as_deref(),
        ]);
        (!value.is_empty()).then_some(value)
    }

    #[must_use]
    pub fn vertex_credentials<'settings>(
        &'settings self,
        request: &'settings ResourceSdkRequestOptions,
    ) -> Option<&'settings SecretString> {
        first_secret([
            request.vertex_credentials.as_ref(),
            self.vertexai_credentials.as_ref(),
        ])
    }
}

fn first_truthy<const N: usize>(values: [Option<&str>; N]) -> &str {
    let candidate = values
        .into_iter()
        .flatten()
        .find(|value| !value.is_empty())
        .unwrap_or("");
    if candidate.is_empty() { "" } else { candidate }
}

fn first_secret<const N: usize>(values: [Option<&SecretString>; N]) -> Option<&SecretString> {
    values
        .into_iter()
        .flatten()
        .find(|value| !value.expose_secret().is_empty())
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
pub struct StructuredTimeout {
    #[serde(default)]
    pub connect: Option<f64>,
    #[serde(default)]
    pub read: Option<f64>,
    #[serde(default)]
    pub write: Option<f64>,
    #[serde(default)]
    pub pool: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FineTuningTimeout {
    Seconds(f64),
    Structured(StructuredTimeout),
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum TimeoutError {
    #[error("timeout must be finite")]
    NonFinite,
    #[error("timeout is not a valid number: {source}")]
    InvalidNumber { source: ParseFloatError },
}

#[must_use]
pub fn supports_httpx_timeout(provider: &str) -> bool {
    let supported = matches!(provider, "openai" | "azure" | "bedrock");
    supported
}

pub fn resolve_fine_tuning_timeout(
    raw: Option<&TimeoutValue>,
    provider: &str,
    structured: Option<StructuredTimeout>,
) -> Result<FineTuningTimeout, TimeoutError> {
    if let Some(timeout) = structured {
        if supports_httpx_timeout(provider) {
            validate_structured(timeout)?;
            return Ok(FineTuningTimeout::Structured(timeout));
        }
        return Ok(FineTuningTimeout::Seconds(finite_or_default(timeout.read)?));
    }
    let seconds = match raw {
        None => 600.0_f64,
        Some(TimeoutValue::Number(value)) if *value == 0.0_f64 => 600.0_f64,
        Some(TimeoutValue::Number(value)) => *value,
        Some(TimeoutValue::String(value)) if value.is_empty() => 600.0_f64,
        Some(TimeoutValue::String(value)) => parse_timeout_string(value)?,
    };
    if !seconds.is_finite() {
        return Err(TimeoutError::NonFinite);
    }
    Ok(FineTuningTimeout::Seconds(seconds))
}

pub fn request_timeout(
    timeout: Option<f64>,
    request_timeout: Option<f64>,
) -> Result<f64, TimeoutError> {
    let selected = timeout
        .filter(|value| *value != 0.0_f64)
        .or_else(|| request_timeout.filter(|value| *value != 0.0_f64))
        .unwrap_or(600.0_f64);
    if selected.is_finite() {
        Ok(selected)
    } else {
        Err(TimeoutError::NonFinite)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum TimeoutValue {
    Number(f64),
    String(String),
}

fn finite_or_default(value: Option<f64>) -> Result<f64, TimeoutError> {
    let timeout = value
        .filter(|number| *number != 0.0_f64)
        .unwrap_or(600.0_f64);
    if timeout.is_finite() {
        Ok(timeout)
    } else {
        Err(TimeoutError::NonFinite)
    }
}

fn parse_timeout_string(value: &str) -> Result<f64, TimeoutError> {
    let trimmed = value.trim();
    let valid_underscores = trimmed
        .chars()
        .zip(trimmed.chars().skip(1))
        .all(|(left, right)| {
            (left != '_' || right.is_ascii_digit()) && (right != '_' || left.is_ascii_digit())
        });
    let candidate = if valid_underscores && !trimmed.starts_with('_') && !trimmed.ends_with('_') {
        trimmed.replace('_', "")
    } else {
        trimmed.to_owned()
    };
    candidate
        .parse::<f64>()
        .map_err(|source| TimeoutError::InvalidNumber { source })
}

fn validate_structured(timeout: StructuredTimeout) -> Result<(), TimeoutError> {
    if [timeout.connect, timeout.read, timeout.write, timeout.pool]
        .into_iter()
        .flatten()
        .all(f64::is_finite)
    {
        Ok(())
    } else {
        Err(TimeoutError::NonFinite)
    }
}
