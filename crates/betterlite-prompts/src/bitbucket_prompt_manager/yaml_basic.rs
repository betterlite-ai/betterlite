use std::str::FromStr as _;

use serde_json::Number;
use thiserror::Error;

use crate::arguments::json::{Json, Object};

#[derive(Debug, Error)]
#[error("'{value}' is not a number")]
pub struct BasicYamlError {
    pub value: String,
    #[source]
    pub source: serde_json::Error,
}

fn parse_number(value: &str) -> Result<Number, BasicYamlError> {
    let (integer, fraction) = value
        .split_once('.')
        .map_or((value, None), |(left, right)| (left, Some(right)));
    let trimmed = integer.trim_start_matches('0');
    let whole = if trimmed.is_empty() { "0" } else { trimmed };
    let text = match fraction {
        Some("") => format!("{whole}.0"),
        Some(digits) => format!("{whole}.{digits}"),
        None => whole.to_owned(),
    };
    Number::from_str(&text).map_err(|source| BasicYamlError {
        value: value.to_owned(),
        source,
    })
}

fn parse_value(value: &str) -> Result<Json, BasicYamlError> {
    match value.to_ascii_lowercase().as_str() {
        "true" => return Ok(Json::Bool(true)),
        "false" => return Ok(Json::Bool(false)),
        _ => {}
    }
    let digits_only = value.replace('.', "");
    if !digits_only.is_empty() && digits_only.bytes().all(|byte| byte.is_ascii_digit()) {
        return parse_number(value).map(Json::Number);
    }
    Ok(Json::Text(value.trim_matches(['"', '\'']).to_owned()))
}

pub fn parse_yaml_basic(yaml: &str) -> Result<Object, BasicYamlError> {
    let mut result = Object::new();
    for raw_line in yaml.split('\n') {
        let line = raw_line.trim();
        if line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let parsed = parse_value(value.trim())?;
        if result.insert(key.trim().to_owned(), parsed).is_some() {
            tracing::warn!(
                key = key.trim(),
                "frontmatter key repeated, the last value wins"
            );
        }
    }
    Ok(result)
}
