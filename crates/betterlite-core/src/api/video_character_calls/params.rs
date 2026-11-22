use betterlite_http::interactions::base::Params;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::error::CharacterCallError;

// GenericLiteLLMParams(**kwargs): the caller's keyword values as the transport reads them
pub(crate) fn request_params(
    extra: &IndexMap<String, Json>,
    litellm_call_id: Option<&str>,
) -> Params {
    let mut params = Params::new();
    if let Some(call_id) = litellm_call_id {
        params.insert(
            "litellm_call_id".to_owned(),
            JsonValue::Text(call_id.to_owned()),
        );
    }
    for (name, value) in extra {
        params.insert(name.clone(), json_value(value));
    }
    params
}

// kwargs.get("user"): absent or null is no user, text is the user, and anything else has the wrong type
pub(crate) fn user(extra: &IndexMap<String, Json>) -> Result<Option<String>, CharacterCallError> {
    match extra.get("user") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(user)) => Ok(Some(user.clone())),
        Some(_) => Err(CharacterCallError::FieldType { field: "user" }),
    }
}

fn json_value(value: &Json) -> JsonValue {
    match value {
        Json::Null => JsonValue::Null,
        Json::Bool(flag) => JsonValue::Bool(*flag),
        Json::Number(number) => JsonValue::Number(number.clone()),
        Json::Text(text) => JsonValue::Text(text.clone()),
        Json::List(items) => JsonValue::Array(items.iter().map(json_value).collect()),
        Json::Object(object) => JsonValue::Object(
            object
                .iter()
                .map(|(name, value)| (name.clone(), json_value(value)))
                .collect(),
        ),
    }
}
