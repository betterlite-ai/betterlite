use betterlite_context::provider_message_primitives::{
    ChoicePolicy, FormatParams, FormatSupport, NestedSchema, ResponseFormat, Schema, ToolParam,
    add_response_format,
};
use betterlite_prompts::arguments::json::{Json, Object};

use super::SUPPORTED_OPENAI_PARAMS;
use super::error::PolicyError;

const GPT_REASONING_SERIES_MARKERS: [&str; 2] = ["gpt-5", "gpt-6"];
const SUPPORTED_RESPONSE_YEAR: i64 = 2024;
const SUPPORTED_RESPONSE_MONTH: i64 = 8;

#[derive(Debug, Clone, Copy)]
pub struct MapPolicy<'request> {
    pub model: &'request str,
    pub api_version: &'request str,
    pub drop_params: bool,
    pub global_drop_params: bool,
}

#[must_use]
pub fn requires_max_completion_tokens(model: &str) -> bool {
    GPT_REASONING_SERIES_MARKERS
        .iter()
        .any(|marker| model.contains(*marker))
        || model.contains("gpt5_series")
}

// Python's re.sub(r"(\d)-(\d)", r"\1.\2") consumes both digits, so "4-1-2" normalizes to "4.1-2"
#[must_use]
pub fn normalize_model_digits(model: &str) -> String {
    let characters: Vec<char> = model.chars().collect();
    let mut normalized = String::with_capacity(model.len());
    let mut index = 0_usize;
    while let Some(character) = characters.get(index) {
        let hyphen = characters.get(index.saturating_add(1));
        let after = characters.get(index.saturating_add(2));
        let dotted = character.is_ascii_digit()
            && hyphen == Some(&'-')
            && after.is_some_and(char::is_ascii_digit);
        normalized.push(*character);
        if dotted {
            normalized.push('.');
            if let Some(digit) = after {
                normalized.push(*digit);
            }
            index = index.saturating_add(3);
        } else {
            index = index.saturating_add(1);
        }
    }
    normalized
}

#[must_use]
pub fn supports_response_format(model: &str) -> bool {
    let normalized = normalize_model_digits(model);
    let legacy_name = normalized.contains("gpt-3.5") || model.contains("gpt-35");
    !legacy_name
}

pub fn format_supported_for_version(year: &str, month: &str) -> Result<bool, PolicyError> {
    let api_year = api_version_number(year, "year")?;
    let api_month = api_version_number(month, "month")?;
    Ok(api_year > SUPPORTED_RESPONSE_YEAR
        || (api_year == SUPPORTED_RESPONSE_YEAR && api_month >= SUPPORTED_RESPONSE_MONTH))
}

fn api_version_number(text: &str, field: &'static str) -> Result<i64, PolicyError> {
    let trimmed = text.trim();
    let (negative, digits) = trimmed.strip_prefix('-').map_or_else(
        || (false, trimmed.strip_prefix('+').unwrap_or(trimmed)),
        |rest| (true, rest),
    );
    let invalid = || PolicyError::ApiVersionNumber {
        field,
        value: text.to_owned(),
    };
    let mut magnitude = 0_i64;
    let mut saw_digit = false;
    for character in digits.chars() {
        if character == '_' {
            if !saw_digit {
                return Err(invalid());
            }
            saw_digit = false;
            continue;
        }
        let Some(digit) = character.to_digit(10) else {
            return Err(invalid());
        };
        magnitude = magnitude
            .checked_mul(10)
            .and_then(|value| value.checked_add(i64::from(digit)))
            .ok_or_else(invalid)?;
        saw_digit = true;
    }
    if !saw_digit {
        return Err(invalid());
    }
    if negative {
        magnitude.checked_neg().ok_or_else(invalid)
    } else {
        Ok(magnitude)
    }
}

fn date_components(api_version: &str) -> Option<(&str, &str, &str)> {
    let mut components = api_version.split('-');
    let year = components.next()?;
    let month = components.next()?;
    let day = components.next()?;
    Some((year, month, day))
}

pub fn map_openai_params(
    non_default: &Object,
    optional: &mut Object,
    policy: &MapPolicy<'_>,
) -> Result<(), PolicyError> {
    let renames_max_tokens = requires_max_completion_tokens(policy.model);
    let date = date_components(policy.api_version);
    for (param, value) in non_default {
        match param.as_str() {
            "max_tokens" if renames_max_tokens => {
                optional
                    .entry(String::from("max_completion_tokens"))
                    .or_insert_with(|| value.clone());
            }
            "tool_choice" => set_tool_choice(optional, value, policy, date)?,
            "response_format" if matches!(value, Json::Object(_)) => {
                apply_response_format(optional, value, policy, date)?;
            }
            "tools" if matches!(value, Json::List(_)) => extend_tools(optional, value)?,
            name if SUPPORTED_OPENAI_PARAMS.contains(&name) => {
                optional.insert(param.clone(), value.clone());
            }
            _ => {}
        }
    }
    Ok(())
}

fn set_tool_choice(
    optional: &mut Object,
    value: &Json,
    policy: &MapPolicy<'_>,
    date: Option<(&str, &str, &str)>,
) -> Result<(), PolicyError> {
    let Some((year, month, day)) = date else {
        optional.insert(String::from("tool_choice"), value.clone());
        return Ok(());
    };
    let before_supported_version = year < "2023"
        || (year == "2023" && month < "12")
        || (year == "2023" && month == "12" && day < "01");
    if before_supported_version {
        if policy.global_drop_params || policy.drop_params {
            return Ok(());
        }
        return Err(PolicyError::ToolChoiceUnsupported {
            api_version: policy.api_version.to_owned(),
        });
    }
    let required_unsupported =
        matches!(value, Json::Text(text) if text == "required") && year == "2024" && month <= "05";
    if required_unsupported {
        if policy.global_drop_params || policy.drop_params {
            return Ok(());
        }
        return Err(PolicyError::RequiredToolChoice {
            api_version: policy.api_version.to_owned(),
        });
    }
    optional.insert(String::from("tool_choice"), value.clone());
    Ok(())
}

fn apply_response_format(
    optional: &mut Object,
    value: &Json,
    policy: &MapPolicy<'_>,
    date: Option<(&str, &str, &str)>,
) -> Result<(), PolicyError> {
    let Json::Object(format) = value else {
        return Ok(());
    };
    let version_supported = date.map_or(Ok(true), |(year, month, _day)| {
        format_supported_for_version(year, month)
    })?;
    let native = version_supported && supports_response_format(policy.model);
    let existing_tools = optional.get("tools");
    let malformed_tools = existing_tools.is_some_and(|tools| !matches!(tools, Json::List(_)));
    let tools = match existing_tools {
        Some(Json::List(items)) => Some(items.iter().cloned().map(ToolParam::Supplied).collect()),
        Some(_) | None => None,
    };
    let mut params = FormatParams {
        tools,
        tool_choice: None,
        json_mode: None,
        response_format: None,
    };
    let support = if native {
        FormatSupport::Native
    } else {
        FormatSupport::ToolCall
    };
    add_response_format(
        &mut params,
        response_format_record(format),
        support,
        ChoicePolicy::Enforce,
    )
    .map_err(|source| PolicyError::ResponseFormat { source })?;
    if malformed_tools && params.json_mode == Some(true) {
        return Err(PolicyError::ToolsNotList);
    }
    merge_format_params(optional, &params)
}

fn merge_format_params(
    optional: &mut Object,
    params: &FormatParams<Json>,
) -> Result<(), PolicyError> {
    let encoded = serde_json::to_value(params).map_err(|source| PolicyError::Encode { source })?;
    let produced =
        serde_json::from_value(encoded).map_err(|source| PolicyError::Encode { source })?;
    if let Json::Object(fields) = produced {
        optional.extend(fields);
    }
    Ok(())
}

fn response_format_record(format: &Object) -> ResponseFormat<Json> {
    let response_schema = format.get("response_schema").map(schema_record);
    let json_schema = format.get("json_schema").map(|nested| match nested {
        Json::Object(fields) => NestedSchema::Object(fields.get("schema").map(schema_record)),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            NestedSchema::NotObject
        }
    });
    ResponseFormat {
        value: Json::Object(format.clone()),
        response_schema,
        json_schema,
    }
}

fn schema_record(value: &Json) -> Schema<Json> {
    if value.is_truthy() {
        Schema::Truthy(value.clone())
    } else {
        Schema::Falsy
    }
}

fn extend_tools(optional: &mut Object, tools: &Json) -> Result<(), PolicyError> {
    let Json::List(incoming) = tools else {
        return Ok(());
    };
    if !optional.contains_key("tools") {
        optional.insert(String::from("tools"), tools.clone());
        return Ok(());
    }
    let Some(Json::List(existing)) = optional.get_mut("tools") else {
        return Err(PolicyError::ToolsNotList);
    };
    existing.extend(incoming.iter().cloned());
    Ok(())
}
