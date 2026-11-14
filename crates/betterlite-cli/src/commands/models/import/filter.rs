use betterlite_models::controls::json_value::JsonValue;
use fancy_regex::Regex;

use crate::commands::models::ModelsError;
use crate::commands::models::value::ModelObject;

pub(super) const MODEL_REGEX_FLAG: &str = "--only-models-matching-regex";
pub(super) const ACCESS_REGEX_FLAG: &str = "--only-access-groups-matching-regex";

pub(super) fn compile(
    pattern: Option<&str>,
    flag: &'static str,
) -> Result<Option<Regex>, ModelsError> {
    match pattern {
        None => Ok(None),
        Some(pattern) => Regex::new(pattern)
            .map(Some)
            .map_err(|source| ModelsError::Regex { flag, source }),
    }
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
    let params = match model.get("litellm_params") {
        Some(JsonValue::Object(params)) => params,
        None | Some(JsonValue::Null) => return Ok(false),
        Some(_) => {
            return Err(ModelsError::EntryField {
                index,
                field: "litellm_params",
            });
        }
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
    let info = match model.get("model_info") {
        Some(JsonValue::Object(info)) => Some(info),
        None | Some(JsonValue::Null) => None,
        Some(_) => {
            return Err(ModelsError::EntryField {
                index,
                field: "model_info",
            });
        }
    };
    let Some(access_regex) = access_regex else {
        return Ok(true);
    };
    let Some(JsonValue::Array(groups)) = info.and_then(|info| info.get("access_groups")) else {
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
