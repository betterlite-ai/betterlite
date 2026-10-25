use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use std::num::ParseFloatError;

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
    pub vertex_credentials: Option<SecretString>,
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

impl ResourceSdkSettings {
    #[must_use]
    pub fn openai_base(&self) -> String {
        let base = first_truthy([
            self.api_base.as_deref(),
            self.openai_base_url.as_deref(),
            self.openai_api_base.as_deref(),
            Some("https://api.openai.com/v1"),
        ]);
        base.to_owned()
    }

    #[must_use]
    pub fn openai_api_key(&self) -> Option<&SecretString> {
        let key = first_secret([
            self.api_key.as_ref(),
            self.openai_key.as_ref(),
            self.openai_api_key.as_ref(),
        ]);
        key
    }

    #[must_use]
    pub fn azure_api_key(&self) -> Option<&SecretString> {
        let key = first_secret([
            self.api_key.as_ref(),
            self.azure_key.as_ref(),
            self.azure_openai_api_key.as_ref(),
            self.azure_api_key.as_ref(),
        ]);
        key
    }

    #[must_use]
    pub fn azure_base(&self) -> Option<&str> {
        let base = first_truthy_ref([self.api_base.as_deref(), self.azure_api_base.as_deref()]);
        base
    }

    #[must_use]
    pub fn azure_version(&self) -> Option<&str> {
        let version = first_truthy_ref([
            self.api_version.as_deref(),
            self.azure_api_version.as_deref(),
        ]);
        version
    }

    #[must_use]
    pub fn organization(&self) -> Option<&str> {
        let organization = first_truthy_ref([
            self.organization.as_deref(),
            self.openai_organization.as_deref(),
        ]);
        organization
    }

    #[must_use]
    pub fn vertex_base(&self) -> &str {
        let base = self.api_base.as_deref().unwrap_or("");
        base
    }

    #[must_use]
    pub fn vertex_project(&self) -> Option<&str> {
        let project = first_truthy_ref([
            self.vertex_project.as_deref(),
            self.vertexai_project.as_deref(),
        ]);
        project
    }

    #[must_use]
    pub fn vertex_location(&self) -> Option<&str> {
        let location = first_truthy_ref([
            self.vertex_location.as_deref(),
            self.vertexai_location.as_deref(),
        ]);
        location
    }

    #[must_use]
    pub fn vertex_credentials(&self) -> Option<&SecretString> {
        let credentials = first_secret([
            self.vertex_credentials.as_ref(),
            self.vertexai_credentials.as_ref(),
        ]);
        credentials
    }
}

fn first_truthy<const N: usize>(values: [Option<&str>; N]) -> &str {
    let value = values
        .into_iter()
        .flatten()
        .find(|value| !value.is_empty())
        .unwrap_or("");
    value
}

fn first_truthy_ref<const N: usize>(values: [Option<&str>; N]) -> Option<&str> {
    let value = values.into_iter().flatten().find(|value| !value.is_empty());
    value
}

fn first_secret<const N: usize>(values: [Option<&SecretString>; N]) -> Option<&SecretString> {
    let secret = values
        .into_iter()
        .flatten()
        .find(|value| !value.expose_secret().is_empty());
    secret
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

#[derive(Debug, Eq, PartialEq)]
pub enum TimeoutError {
    NonFinite,
    InvalidNumber { source: ParseFloatError },
}

#[must_use]
pub fn supports_httpx_timeout(provider: &str) -> bool {
    let supported = matches!(provider, "openai" | "azure" | "bedrock");
    supported
}

pub fn resolve_fine_tuning_timeout(
    raw: Option<&str>,
    provider: &str,
    structured: Option<StructuredTimeout>,
) -> Result<FineTuningTimeout, TimeoutError> {
    if let Some(timeout) = structured {
        if supports_httpx_timeout(provider) {
            validate_structured(timeout)?;
            return Ok(FineTuningTimeout::Structured(timeout));
        }
        return Ok(FineTuningTimeout::Seconds(finite_or_default(timeout.read)));
    }
    let seconds = raw
        .filter(|value| !value.is_empty() && *value != "0")
        .map(|value| value.replace('_', "").parse::<f64>())
        .transpose()
        .map_err(|source| TimeoutError::InvalidNumber { source })?
        .unwrap_or(600.0_f64);
    if !seconds.is_finite() {
        return Err(TimeoutError::NonFinite);
    }
    Ok(FineTuningTimeout::Seconds(seconds))
}

#[must_use]
pub fn request_timeout(timeout: Option<f64>, request_timeout: Option<f64>) -> f64 {
    let selected = timeout
        .filter(|value| *value != 0.0_f64)
        .or_else(|| request_timeout.filter(|value| *value != 0.0_f64))
        .unwrap_or(600.0_f64);
    selected
}

fn finite_or_default(value: Option<f64>) -> f64 {
    let timeout = value
        .filter(|number| number.is_finite() && *number != 0.0_f64)
        .unwrap_or(600.0_f64);
    timeout
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
