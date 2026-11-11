use betterlite_anthropic::skills::wire::ListSkillsParams;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::call::ListSkillsCall;
use super::error::ListSkillsError;

// LiteLLM fills ListSkillsParams from the named arguments, then lets extra_query overlay whatever it carries
pub fn list_params(call: &ListSkillsCall<'_>) -> Result<ListSkillsParams, ListSkillsError> {
    let mut params = ListSkillsParams {
        limit: call.limit,
        page: call.page.map(str::to_owned),
        source: call.source.map(str::to_owned),
    };
    if let Some(extra_query) = call.params.extra_query.as_ref() {
        overlay(&mut params, extra_query)?;
    }
    Ok(params)
}

fn overlay(
    params: &mut ListSkillsParams,
    extra_query: &IndexMap<String, Json>,
) -> Result<(), ListSkillsError> {
    for (name, value) in extra_query {
        match name.as_str() {
            "limit" => params.limit = limit_param(value)?,
            "page" => params.page = text_param(value, name)?,
            "source" => params.source = text_param(value, name)?,
            // the transform reads only limit, page and source, so LiteLLM's other extra_query keys never change the request
            _ => {}
        }
    }
    Ok(())
}

fn limit_param(value: &Json) -> Result<Option<i64>, ListSkillsError> {
    match value {
        Json::Null => Ok(None),
        Json::Number(number) => {
            number
                .as_i64()
                .map(Some)
                .ok_or_else(|| ListSkillsError::FieldType {
                    name: "limit".to_owned(),
                })
        }
        Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => {
            Err(ListSkillsError::FieldType {
                name: "limit".to_owned(),
            })
        }
    }
}

fn text_param(value: &Json, name: &str) -> Result<Option<String>, ListSkillsError> {
    match value {
        Json::Null => Ok(None),
        Json::Text(text) => Ok(Some(text.clone())),
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            Err(ListSkillsError::FieldType {
                name: name.to_owned(),
            })
        }
    }
}
