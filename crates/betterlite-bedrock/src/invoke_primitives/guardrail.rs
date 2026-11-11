use serde::Deserialize;
use serde_json::value::RawValue;
use thiserror::Error;

const IDENTIFIER: &str = "X-Amzn-Bedrock-GuardrailIdentifier";
const VERSION: &str = "X-Amzn-Bedrock-GuardrailVersion";
const TRACE: &str = "X-Amzn-Bedrock-Trace";

#[derive(Debug, Error)]
pub enum GuardrailError {
    #[error(
        "Invalid guardrailConfig={raw}. Expected format: {{'guardrailIdentifier': str, 'guardrailVersion': str, 'trace': 'enabled'|'disabled'|'enabled_full'}}. Error: {source}"
    )]
    Invalid {
        raw: String,
        #[source]
        source: serde_json::Error,
    },
    #[error(
        "guardrailConfig={raw} is missing 'guardrailIdentifier'. Expected format: {{'guardrailIdentifier': str, 'guardrailVersion': str, 'trace': 'enabled'|'disabled'|'enabled_full'}}"
    )]
    MissingIdentifier { raw: String },
}

impl GuardrailError {
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::Invalid { .. } | Self::MissingIdentifier { .. } => 400,
        }
    }
}

#[derive(Deserialize)]
struct GuardrailConfig {
    #[serde(rename = "guardrailIdentifier")]
    identifier: Option<String>,
    #[serde(rename = "guardrailVersion")]
    version: Option<String>,
    trace: Option<Trace>,
}

#[derive(Deserialize)]
enum Trace {
    #[serde(rename = "enabled")]
    Enabled,
    #[serde(rename = "disabled")]
    Disabled,
    #[serde(rename = "enabled_full")]
    EnabledFull,
}

pub fn guardrail_headers(raw: &RawValue) -> Result<Vec<(String, String)>, GuardrailError> {
    let config: GuardrailConfig =
        serde_json::from_str(raw.get()).map_err(|source| GuardrailError::Invalid {
            raw: raw.get().to_owned(),
            source,
        })?;
    let Some(identifier) = config.identifier else {
        return Err(GuardrailError::MissingIdentifier {
            raw: raw.get().to_owned(),
        });
    };
    let mut headers = vec![(IDENTIFIER.to_owned(), identifier)];
    if let Some(version) = config.version {
        headers.push((VERSION.to_owned(), version));
    }
    if let Some(trace) = config.trace {
        let value = match trace {
            Trace::Enabled => "ENABLED",
            Trace::Disabled => "DISABLED",
            Trace::EnabledFull => "ENABLED_FULL",
        };
        headers.push((TRACE.to_owned(), value.to_owned()));
    }
    Ok(headers)
}
