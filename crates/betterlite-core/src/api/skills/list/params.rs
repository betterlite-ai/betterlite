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
            "limit" => {
                params.limit = match value {
                    Json::Null => None,
                    Json::Number(number) => {
                        Some(number.as_i64().ok_or_else(|| ListSkillsError::FieldType {
                            name: "limit".to_owned(),
                        })?)
                    }
                    Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => {
                        return Err(ListSkillsError::FieldType {
                            name: "limit".to_owned(),
                        });
                    }
                };
            }
            "page" => {
                params.page = match value {
                    Json::Null => None,
                    Json::Text(text) => Some(text.clone()),
                    Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                        return Err(ListSkillsError::FieldType { name: name.clone() });
                    }
                };
            }
            "source" => {
                params.source = match value {
                    Json::Null => None,
                    Json::Text(text) => Some(text.clone()),
                    Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                        return Err(ListSkillsError::FieldType { name: name.clone() });
                    }
                };
            }
            // the transform reads only limit, page and source, so LiteLLM's other extra_query keys never change the request
            _ => {}
        }
    }
    Ok(())
}
