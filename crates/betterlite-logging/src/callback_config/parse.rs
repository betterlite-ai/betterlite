use std::slice::from_ref;

use indexmap::IndexMap;
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::Number;

use super::error::{ParseError, PayloadError, ValidationError};
use super::family::{SPAN_SCOPE_VAR, conflicting_span_scope_error};
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
    Flag(bool),
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
    #[serde(rename = "callback_name")]
    name: String,
    #[serde(rename = "callback_type")]
    kind: Option<CallbackType>,
    #[serde(rename = "callback_vars")]
    variables: IndexMap<String, RawValue>,
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
    pub callback_vars: CallbackVars,
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
            RawValue::Flag(true) => "True".to_owned(),
            RawValue::Flag(false) => "False".to_owned(),
            RawValue::Null => "None".to_owned(),
            RawValue::Other(_) => return Err(ParseError::VariableType { var }),
        };
        variables.insert(var, text);
    }
    Ok(variables)
}

pub fn parse_logging_metadata(bytes: &[u8]) -> Result<Vec<LoggingEntry>, ParseError> {
    let parsed: Option<RawMetadata> = serde_json::from_slice(bytes)?;
    let Some(Lenient::Value(raw_entries)) = parsed.and_then(|found| found.logging) else {
        return Ok(Vec::new());
    };
    let mut entries = Vec::new();
    for candidate in raw_entries {
        let Lenient::Value(raw) = candidate else {
            entries.push(LoggingEntry {
                callback_name: None,
                callback_vars: CallbackVars::new(),
            });
            continue;
        };
        entries.push(LoggingEntry {
            callback_name: match raw.callback_name {
                Some(Lenient::Value(name)) => Some(name),
                Some(Lenient::Other(_)) | None => None,
            },
            callback_vars: match raw.callback_vars {
                Some(Lenient::Value(vars)) => text_variables(vars)?,
                Some(Lenient::Other(_)) | None => CallbackVars::new(),
            },
        });
    }
    Ok(entries)
}

pub fn parse_team_callback(bytes: &[u8]) -> Result<TeamCallback, ParseError> {
    let raw: RawTeamCallback = serde_json::from_slice(bytes)?;
    Ok(TeamCallback {
        callback_name: raw.name,
        callback_type: raw.kind.unwrap_or_default(),
        callback_vars: text_variables(raw.variables)?,
    })
}

pub fn logging_metadata_config_error(
    entries: &[LoggingEntry],
    flags: FeatureFlags,
) -> Result<(), ValidationError> {
    for entry in entries {
        if let Some(name) = &entry.callback_name {
            callback_config_error(Some(name), &entry.callback_vars, flags)?;
        }
    }
    let mut first_scoped: Option<&CallbackVars> = None;
    for entry in entries {
        conflicting_span_scope_error(&entry.callback_vars, first_scoped.map_or(&[], from_ref))?;
        first_scoped = first_scoped.or_else(|| {
            entry
                .callback_vars
                .contains_key(SPAN_SCOPE_VAR)
                .then_some(&entry.callback_vars)
        });
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
