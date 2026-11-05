use indexmap::IndexMap;
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::Number;

use super::error::{ParseError, PayloadError, ValidationError};
use super::family::conflicting_span_scope_error;
use super::options::callback_config_error;
use super::{CallbackVars, FeatureFlags};

// A value of another shape is skipped, which is how LiteLLM treats the key metadata it does not recognize.
#[derive(Deserialize)]
#[serde(untagged)]
enum Lenient<T> {
    Value(T),
    Other(IgnoredAny),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawValue {
    Text(String),
    Number(Number),
    Null,
    Other(IgnoredAny),
}

#[derive(Deserialize)]
struct RawEntry {
    callback_name: Option<Lenient<String>>,
    callback_vars: Option<Lenient<IndexMap<String, RawValue>>>,
}

#[derive(Deserialize)]
struct RawMetadata {
    logging: Option<Lenient<Vec<Lenient<RawEntry>>>>,
}

#[derive(Deserialize)]
struct RawTeamCallback {
    callback_name: String,
    callback_type: Option<CallbackType>,
    callback_vars: IndexMap<String, RawValue>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallbackType {
    Success,
    Failure,
    #[default]
    SuccessAndFailure,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoggingEntry {
    pub callback_name: Option<String>,
    pub callback_vars: Option<CallbackVars>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamCallback {
    pub callback_name: String,
    pub callback_type: CallbackType,
    pub callback_vars: CallbackVars,
}

// LiteLLM str()-coerces every variable before validating, and a JSON null becomes the text "None".
fn text_variables(raw: IndexMap<String, RawValue>) -> Result<CallbackVars, ParseError> {
    let mut variables = CallbackVars::new();
    for (var, value) in raw {
        let text = match value {
            RawValue::Text(text) => text,
            RawValue::Number(number) => number.to_string(),
            RawValue::Null => "None".to_owned(),
            RawValue::Other(_) => return Err(ParseError::VariableType { var }),
        };
        variables.insert(var, text);
    }
    Ok(variables)
}

pub fn parse_logging_metadata(bytes: &[u8]) -> Result<Vec<LoggingEntry>, ParseError> {
    let metadata: Option<RawMetadata> = serde_json::from_slice(bytes)?;
    let Some(Lenient::Value(raw_entries)) = metadata.and_then(|metadata| metadata.logging) else {
        return Ok(Vec::new());
    };
    let mut entries = Vec::new();
    for raw in raw_entries {
        let Lenient::Value(raw) = raw else {
            entries.push(LoggingEntry {
                callback_name: None,
                callback_vars: None,
            });
            continue;
        };
        entries.push(LoggingEntry {
            callback_name: match raw.callback_name {
                Some(Lenient::Value(name)) => Some(name),
                Some(Lenient::Other(_)) | None => None,
            },
            callback_vars: match raw.callback_vars {
                Some(Lenient::Value(vars)) => Some(text_variables(vars)?),
                Some(Lenient::Other(_)) | None => None,
            },
        });
    }
    Ok(entries)
}

pub fn parse_team_callback(bytes: &[u8]) -> Result<TeamCallback, ParseError> {
    let raw: RawTeamCallback = serde_json::from_slice(bytes)?;
    Ok(TeamCallback {
        callback_name: raw.callback_name,
        callback_type: raw.callback_type.unwrap_or_default(),
        callback_vars: text_variables(raw.callback_vars)?,
    })
}

pub fn logging_metadata_config_error(
    entries: &[LoggingEntry],
    flags: FeatureFlags,
) -> Result<(), ValidationError> {
    for entry in entries {
        if let (Some(name), Some(vars)) = (&entry.callback_name, &entry.callback_vars) {
            callback_config_error(Some(name), vars, flags)?;
        }
    }
    let mut earlier = Vec::new();
    for entry in entries {
        let vars = entry.callback_vars.clone().unwrap_or_default();
        conflicting_span_scope_error(&vars, &earlier)?;
        earlier.push(vars);
    }
    Ok(())
}

pub fn validate_logging_metadata(bytes: &[u8], flags: FeatureFlags) -> Result<(), PayloadError> {
    let entries = parse_logging_metadata(bytes)?;
    logging_metadata_config_error(&entries, flags)?;
    Ok(())
}

pub fn validate_team_callback(bytes: &[u8], flags: FeatureFlags) -> Result<(), PayloadError> {
    let callback = parse_team_callback(bytes)?;
    Ok(callback_config_error(
        Some(&callback.callback_name),
        &callback.callback_vars,
        flags,
    )?)
}
