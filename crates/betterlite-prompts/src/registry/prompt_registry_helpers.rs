use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use betterlite_models::prompt_spec::PromptSpec;

pub const DEFAULT_PROMPT_ENVIRONMENT: &str = "development";
pub const ENVIRONMENT_PRECEDENCE: [&str; 3] = ["production", "staging", "development"];

#[must_use]
pub fn get_base_prompt_id(prompt_id: &str) -> String {
    if let Some((base, _)) = prompt_id.split_once(".v") {
        return base.to_owned();
    }
    if let Some((base, _)) = prompt_id.split_once("_v") {
        return base.to_owned();
    }
    prompt_id.to_owned()
}

#[must_use]
pub fn get_version_number(prompt_id: &str) -> i64 {
    if let Some(version) = version_suffix(prompt_id, ".v") {
        return version;
    }
    if let Some(version) = version_suffix(prompt_id, "_v") {
        return version;
    }
    1
}

fn version_suffix(prompt_id: &str, marker: &str) -> Option<i64> {
    let (_, rest) = prompt_id.split_once(marker)?;
    rest.split_once(marker)
        .map_or(rest, |(head, _)| head)
        .parse::<i64>()
        .ok()
}

#[must_use]
pub fn prompt_environment_or_default(environment: Option<&str>) -> String {
    environment.map_or_else(
        || DEFAULT_PROMPT_ENVIRONMENT.to_owned(),
        |requested| requested.to_owned(),
    )
}

#[must_use]
pub fn registry_key_for_prompt(prompt: &PromptSpec) -> String {
    format!(
        "{}::{}",
        prompt.prompt_id,
        prompt_environment_or_default(Some(prompt.environment.as_str()))
    )
}

#[must_use]
pub fn parse_prompt_version(raw_version: Option<&Json>) -> Option<i64> {
    let version = raw_version?;
    match version {
        Json::Number(number) => number.as_i64(),
        Json::Text(text) if text.bytes().all(|byte| byte.is_ascii_digit()) => {
            text.parse::<i64>().ok()
        }
        Json::Bool(_) | Json::Null | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
    }
}

#[must_use]
pub fn spec_version(prompt: &PromptSpec) -> i64 {
    prompt
        .version
        .unwrap_or_else(|| get_version_number(&prompt.prompt_id))
}

#[must_use]
pub fn default_serve_environment(prompts: &[&PromptSpec]) -> String {
    let present: BTreeSet<&str> = prompts
        .iter()
        .map(|prompt| prompt.environment.as_str())
        .collect();
    for environment in ENVIRONMENT_PRECEDENCE {
        if present.contains(environment) {
            return environment.to_owned();
        }
    }
    present
        .into_iter()
        .next()
        .unwrap_or(DEFAULT_PROMPT_ENVIRONMENT)
        .to_owned()
}

mod tests;
