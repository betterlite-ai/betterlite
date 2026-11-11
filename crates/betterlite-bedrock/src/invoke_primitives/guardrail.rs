use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::value::RawValue;
use thiserror::Error;

const IDENTIFIER: &str = "X-Amzn-Bedrock-GuardrailIdentifier";
const VERSION: &str = "X-Amzn-Bedrock-GuardrailVersion";
const TRACE: &str = "X-Amzn-Bedrock-Trace";

#[derive(Debug, Error)]
pub enum GuardrailError {
    #[error(
        "Invalid guardrailConfig={raw}. Expected format: {{'guardrailIdentifier': str, 'guardrailVersion': str, 'trace': 'enabled'|'disabled'|'enabled_full'}}. Error: {detail}"
    )]
    Invalid { raw: String, detail: String },
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

pub fn guardrail_headers(raw: &RawValue) -> Result<Vec<(String, String)>, GuardrailError> {
    let text = raw.get();
    let parsed: Json = serde_json::from_str(text).map_err(|source| GuardrailError::Invalid {
        raw: text.to_owned(),
        detail: source.to_string(),
    })?;
    let Json::Object(fields) = parsed else {
        return Err(GuardrailError::Invalid {
            raw: text.to_owned(),
            detail: "expected an object".to_owned(),
        });
    };
    let identifier = text_field(&fields, "guardrailIdentifier", text)?;
    let version = text_field(&fields, "guardrailVersion", text)?;
    let trace = match fields.get("trace") {
        None => None,
        Some(Json::Text(value))
            if matches!(value.as_str(), "enabled" | "disabled" | "enabled_full") =>
        {
            Some(value.to_ascii_uppercase())
        }
        Some(_) => {
            return Err(GuardrailError::Invalid {
                raw: text.to_owned(),
                detail: "trace must be 'enabled', 'disabled' or 'enabled_full'".to_owned(),
            });
        }
    };
    let Some(required_identifier) = identifier else {
        return Err(GuardrailError::MissingIdentifier {
            raw: text.to_owned(),
        });
    };
    let mut headers = vec![(IDENTIFIER.to_owned(), required_identifier.to_owned())];
    if let Some(version_value) = version {
        headers.push((VERSION.to_owned(), version_value.to_owned()));
    }
    if let Some(trace_value) = trace {
        headers.push((TRACE.to_owned(), trace_value));
    }
    Ok(headers)
}

fn text_field<'fields>(
    fields: &'fields IndexMap<String, Json>,
    name: &str,
    raw: &str,
) -> Result<Option<&'fields str>, GuardrailError> {
    match fields.get(name) {
        None => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text)),
        Some(_) => Err(GuardrailError::Invalid {
            raw: raw.to_owned(),
            detail: format!("{name} must be a string"),
        }),
    }
}
