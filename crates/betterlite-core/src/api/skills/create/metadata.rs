use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::error::CreateSkillError;
use super::request::text_field;

// LiteLLM's _get_skill_request_metadata: extra_body metadata wins over kwargs metadata.requester_metadata
pub fn requester_metadata<'values>(
    kwargs: &'values IndexMap<String, Json>,
    extra_body: Option<&'values IndexMap<String, Json>>,
) -> Option<&'values IndexMap<String, Json>> {
    let from_extra_body = extra_body
        .and_then(|body| body.get("metadata"))
        .and_then(object);
    let from_kwargs = kwargs
        .get("metadata")
        .and_then(object)
        .and_then(|metadata| metadata.get("requester_metadata"))
        .and_then(object);
    from_extra_body.or(from_kwargs)
}

// LiteLLM reads description and instructions from the REST form's top-level kwargs, then from extra_body
pub fn hosted_text<'values>(
    kwargs: &'values IndexMap<String, Json>,
    extra_body: Option<&'values IndexMap<String, Json>>,
    name: &'static str,
) -> Result<Option<&'values str>, CreateSkillError> {
    match (text_field(kwargs, name)?, extra_body) {
        (Some(text), _) => Ok(Some(text)),
        (None, Some(body)) => text_field(body, name),
        (None, None) => Ok(None),
    }
}

const fn object(value: &Json) -> Option<&IndexMap<String, Json>> {
    match value {
        Json::Object(map) => Some(map),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}
