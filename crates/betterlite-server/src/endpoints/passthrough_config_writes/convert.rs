use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::endpoints::passthrough_config_reads::record::PassThroughGenericEndpoint;
use crate::endpoints::passthrough_config_reads::record::PassThroughGuardrailSettings;

use super::error::PassthroughWriteError;

const IS_FROM_CONFIG: &str = "is_from_config";
const REQUEST_FIELDS: &str = "request_fields";
const RESPONSE_FIELDS: &str = "response_fields";

fn json_of(value: &JsonData) -> Json {
    match value {
        JsonData::Null => Json::Null,
        JsonData::Bool(flag) => Json::Bool(*flag),
        JsonData::Number(number) => Json::Number(number.clone()),
        JsonData::String(text) => Json::Text(text.clone()),
        JsonData::Array(items) => Json::List(items.iter().map(json_of).collect()),
        JsonData::Object(fields) => Json::Object(
            fields
                .iter()
                .map(|(key, entry)| (key.clone(), json_of(entry)))
                .collect(),
        ),
    }
}

pub(super) fn json_object(fields: &BTreeMap<String, JsonData>) -> JsonObject {
    let mut object = JsonObject::new();
    for (key, value) in fields {
        object.insert(key.clone(), json_of(value));
    }
    object
}

fn text_list(items: Option<&[String]>) -> Json {
    items.map_or(Json::Null, |names| {
        Json::List(names.iter().map(|item| Json::Text(item.clone())).collect())
    })
}

fn guardrail_fields(settings: &PassThroughGuardrailSettings) -> JsonObject {
    let mut fields = JsonObject::new();
    fields.insert(
        REQUEST_FIELDS.to_owned(),
        text_list(settings.request_fields.as_deref()),
    );
    fields.insert(
        RESPONSE_FIELDS.to_owned(),
        text_list(settings.response_fields.as_deref()),
    );
    fields
}

pub(super) fn guardrails_object(
    guardrails: Option<&BTreeMap<String, Option<PassThroughGuardrailSettings>>>,
) -> Option<JsonObject> {
    let configured = guardrails?;
    let mut object = JsonObject::new();
    for (name, entry) in configured {
        let value = entry.as_ref().map_or(Json::Null, |settings| {
            Json::Object(guardrail_fields(settings))
        });
        object.insert(name.clone(), value);
    }
    Some(object)
}

// the entry as LiteLLM stores it: model_dump() without is_from_config, which only the read computes
pub(super) fn stored_value(
    endpoint: &PassThroughGenericEndpoint,
) -> Result<JsonData, PassthroughWriteError> {
    let text = serde_json::to_string(endpoint).map_err(PassthroughWriteError::Encode)?;
    let mut value: JsonData = serde_json::from_str(&text).map_err(PassthroughWriteError::Encode)?;
    if let JsonData::Object(fields) = &mut value {
        fields.retain(|key, _entry| key != IS_FROM_CONFIG);
    }
    Ok(value)
}
