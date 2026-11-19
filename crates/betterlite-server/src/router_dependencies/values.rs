use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::tier_definitions::checks::ClassifierKind;
use serde::Deserialize as _;
use serde::de::value::{Error, StrDeserializer};

// the declared name of one field, or none when the caller holds a value the router itself would refuse
pub(super) fn text(value: Option<&Json>) -> Option<&str> {
    match value {
        Some(Json::Text(text)) => Some(text),
        _ => None,
    }
}

pub(super) const fn object(value: Option<&Json>) -> Option<&JsonObject> {
    match value {
        Some(Json::Object(fields)) => Some(fields),
        _ => None,
    }
}

// LiteLLM compares the raw classifier_type spelling, so an unknown or non-text value classifies as none
pub(super) fn classifier_kind(config: Option<&JsonObject>) -> Option<ClassifierKind> {
    let name = text(config.and_then(|fields| fields.get("classifier_type")))?;
    let declared = StrDeserializer::<Error>::new(name);
    ClassifierKind::deserialize(declared).ok()
}
