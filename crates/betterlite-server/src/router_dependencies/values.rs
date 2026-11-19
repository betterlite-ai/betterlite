use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::tier_definitions::checks::ClassifierKind;
use serde::Deserialize;
use serde::de::value::{Error, StrDeserializer};

// the declared name of one field, or none when the caller holds a value the router itself would refuse
pub(super) fn text(value: Option<&Json>) -> Option<&str> {
    match value {
        Some(Json::Text(text)) => Some(text),
        _ => None,
    }
}

pub(super) fn object(value: Option<&Json>) -> Option<&JsonObject> {
    match value {
        Some(Json::Object(fields)) => Some(fields),
        _ => None,
    }
}

pub(super) fn field<'json>(source: Option<&'json JsonObject>, key: &str) -> Option<&'json Json> {
    let Some(fields) = source else {
        return None;
    };
    fields.get(key)
}

// LiteLLM compares the raw classifier_type spelling, so an unknown or non-text value classifies as none
pub(super) fn classifier_kind(config: Option<&JsonObject>) -> Option<ClassifierKind> {
    let Some(name) = text(field(config, "classifier_type")) else {
        return None;
    };
    ClassifierKind::deserialize(StrDeserializer::<Error>::new(name)).ok()
}
