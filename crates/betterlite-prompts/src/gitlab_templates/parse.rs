use std::str::FromStr as _;

use serde_json::Number;
use serde_saphyr::{DuplicateKeyPolicy, from_str_with_options, options};
use thiserror::Error;

use super::template::{GitLabPromptTemplate, TemplateError};
use crate::arguments::json::{Json, Object};

const FRONTMATTER_FENCE: &str = "---";

// Python falls back to the basic parser when the yaml module cannot be imported; Rust always has a YAML parser.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FrontmatterParser {
    #[default]
    Yaml,
    Basic,
}

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("frontmatter metadata must be a mapping")]
    MetadataNotMapping,
    #[error(transparent)]
    Template(#[from] TemplateError),
}

pub fn parse_prompt_file(
    content: &str,
    prompt_id: &str,
    parser: FrontmatterParser,
) -> Result<GitLabPromptTemplate, ParseError> {
    let (frontmatter, template_content) = split_frontmatter(content);
    let metadata = match (frontmatter.is_empty(), parser) {
        (true, _) => Object::new(),
        (false, FrontmatterParser::Yaml) => yaml_metadata(frontmatter)?,
        (false, FrontmatterParser::Basic) => parse_yaml_basic(frontmatter),
    };
    let template = GitLabPromptTemplate::new(
        prompt_id.to_owned(),
        template_content.to_owned(),
        metadata,
        None,
    )?;
    Ok(template)
}

// Mirrors Python's `content.split("---", 2)`: content without a closing fence is all template.
fn split_frontmatter(content: &str) -> (&str, &str) {
    if content.starts_with(FRONTMATTER_FENCE) {
        let mut parts = content.splitn(3, FRONTMATTER_FENCE);
        if let (Some(_), Some(frontmatter), Some(template)) =
            (parts.next(), parts.next(), parts.next())
        {
            return (frontmatter.trim(), template.trim());
        }
    }
    ("", content)
}

// A YAML error leaves the metadata empty; a document that is not a mapping fails later, at the template.
fn yaml_metadata(frontmatter: &str) -> Result<Object, ParseError> {
    let options = options! {
        duplicate_keys: DuplicateKeyPolicy::LastWins,
        strict_booleans: true,
    };
    let Ok(document) = from_str_with_options::<Json>(frontmatter, options) else {
        return Ok(Object::new());
    };
    match resolve_yaml11_booleans(document) {
        Json::Object(metadata) => Ok(metadata),
        other if !other.is_truthy() => Ok(Object::new()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(ParseError::MetadataNotMapping)
        }
    }
}

// PyYAML's resolver takes exactly these plain spellings as booleans; serde-saphyr's strict mode takes YAML 1.2 true/false only.
const YAML11_TRUE_SPELLINGS: [&str; 6] = ["yes", "Yes", "YES", "on", "On", "ON"];
const YAML11_FALSE_SPELLINGS: [&str; 6] = ["no", "No", "NO", "off", "Off", "OFF"];

fn resolve_yaml11_booleans(value: Json) -> Json {
    match value {
        Json::Text(text) if YAML11_TRUE_SPELLINGS.contains(&text.as_str()) => Json::Bool(true),
        Json::Text(text) if YAML11_FALSE_SPELLINGS.contains(&text.as_str()) => Json::Bool(false),
        Json::Text(text) => Json::Text(text),
        Json::List(items) => Json::List(items.into_iter().map(resolve_yaml11_booleans).collect()),
        Json::Object(fields) => Json::Object(
            fields
                .into_iter()
                .map(|(key, field)| (key, resolve_yaml11_booleans(field)))
                .collect(),
        ),
        Json::Null | Json::Bool(_) | Json::Number(_) => value,
    }
}

#[must_use]
pub fn parse_yaml_basic(yaml_str: &str) -> Object {
    let mut result = Object::new();
    for raw_line in yaml_str.split('\n') {
        let line = raw_line.trim();
        if line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            result.insert(key.trim().to_owned(), basic_value(value.trim()));
        }
    }
    result
}

fn basic_value(value: &str) -> Json {
    let lowered = value.to_lowercase();
    if lowered == "true" || lowered == "false" {
        return Json::Bool(lowered == "true");
    }
    if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) {
        let digits = value.trim_start_matches('0');
        if let Ok(number) = Number::from_str(if digits.is_empty() { "0" } else { digits }) {
            return Json::Number(number);
        }
    }
    let without_dots = value.replace('.', "");
    if !without_dots.is_empty() && without_dots.bytes().all(|byte| byte.is_ascii_digit()) {
        return value
            .parse::<f64>()
            .ok()
            .and_then(Number::from_f64)
            .map_or_else(|| Json::Text(value.to_owned()), Json::Number);
    }
    Json::Text(value.trim_matches(['"', '\'']).to_owned())
}

mod tests;
