use std::collections::BTreeMap;

use serde::Deserialize;
use serde::de::DeserializeOwned;
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
enum Trace {
    #[serde(rename = "enabled")]
    Enabled,
    #[serde(rename = "disabled")]
    Disabled,
    #[serde(rename = "enabled_full")]
    EnabledFull,
}

pub fn guardrail_headers(raw: &RawValue) -> Result<Vec<(String, String)>, GuardrailError> {
    // a struct derive reads a JSON array positionally; a map rejects every non-object up front
    let config: BTreeMap<String, Box<RawValue>> =
        serde_json::from_str(raw.get()).map_err(invalid(raw))?;
    let identifier_field =
        parse_field::<String>(&config, "guardrailIdentifier").map_err(invalid(raw))?;
    let version_field = parse_field::<String>(&config, "guardrailVersion").map_err(invalid(raw))?;
    let trace_field = parse_field::<Trace>(&config, "trace").map_err(invalid(raw))?;
    let Some(identifier) = identifier_field else {
        return Err(GuardrailError::MissingIdentifier {
            raw: raw.get().to_owned(),
        });
    };
    let mut headers = vec![(IDENTIFIER.to_owned(), identifier)];
    if let Some(version) = version_field {
        headers.push((VERSION.to_owned(), version));
    }
    if let Some(trace) = trace_field {
        headers.push((
            TRACE.to_owned(),
            match trace {
                Trace::Enabled => "ENABLED".to_owned(),
                Trace::Disabled => "DISABLED".to_owned(),
                Trace::EnabledFull => "ENABLED_FULL".to_owned(),
            },
        ));
    }
    Ok(headers)
}

fn invalid(raw: &RawValue) -> impl Fn(serde_json::Error) -> GuardrailError + '_ {
    |source| GuardrailError::Invalid {
        raw: raw.get().to_owned(),
        source,
    }
}

fn parse_field<T>(
    config: &BTreeMap<String, Box<RawValue>>,
    name: &str,
) -> Result<Option<T>, serde_json::Error>
where
    T: DeserializeOwned,
{
    config
        .get(name)
        .map(|value| serde_json::from_str(value.get()))
        .transpose()
}
