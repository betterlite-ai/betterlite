use betterlite_models::controls::json_value::JsonValue;
use fancy_regex::Regex;

use crate::commands::models::ModelsError;
use crate::commands::models::value::ModelObject;

use super::field_object;

pub(super) const MODEL_REGEX_FLAG: &str = "--only-models-matching-regex";
pub(super) const ACCESS_REGEX_FLAG: &str = "--only-access-groups-matching-regex";

pub(super) fn compile(
    pattern: Option<&str>,
    flag: &'static str,
) -> Result<Option<Regex>, ModelsError> {
    let Some(text) = pattern else {
        return Ok(None);
    };
    Regex::new(text)
        .map(Some)
        .map_err(|source| ModelsError::Regex { flag, source })
}

pub(super) fn filter_model(
    model: &ModelObject,
    model_regex: Option<&Regex>,
    access_regex: Option<&Regex>,
    index: usize,
) -> Result<bool, ModelsError> {
    if model
        .get("model_name")
        .is_none_or(|value| matches!(value, JsonValue::Null))
    {
        return Ok(false);
    }
    let Some(params) = field_object(model, "litellm_params", index)? else {
        return Ok(false);
    };
    let Some(JsonValue::Text(model_id)) = params.get("model") else {
        return Ok(false);
    };
    if let Some(regex) = model_regex
        && !regex
            .is_match(model_id)
            .map_err(|source| ModelsError::RegexMatch {
                flag: MODEL_REGEX_FLAG,
                source,
            })?
    {
        return Ok(false);
    }
    let info = field_object(model, "model_info", index)?;
    let Some(regex) = access_regex else {
        return Ok(true);
    };
    groups_match(info, regex)
}

fn groups_match(info: Option<&ModelObject>, access_regex: &Regex) -> Result<bool, ModelsError> {
    let Some(JsonValue::Array(groups)) = info.and_then(|object| object.get("access_groups")) else {
        return Ok(false);
    };
    for group in groups {
        let JsonValue::Text(text) = group else {
            continue;
        };
        let matched = access_regex
            .is_match(text)
            .map_err(|source| ModelsError::RegexMatch {
                flag: ACCESS_REGEX_FLAG,
                source,
            })?;
        if matched {
            return Ok(true);
        }
    }
    Ok(false)
}
