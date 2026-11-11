use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;

use crate::fields::payload::StandardLoggingPayload;

// promptlayer reads the model, the messages, the optional params and the metadata out of the callback kwargs
pub(super) fn callback_kwargs(payload: &StandardLoggingPayload) -> Json {
    let mut fields = IndexMap::new();
    fields.insert("model".to_owned(), Json::Text(payload.model.clone()));
    if let Some(messages) = payload.messages.as_ref() {
        fields.insert("messages".to_owned(), messages.clone());
    }
    fields.insert(
        "optional_params".to_owned(),
        Json::Object(payload.model_parameters.clone()),
    );
    fields.insert(
        "litellm_params".to_owned(),
        Json::Object(IndexMap::from([(
            "metadata".to_owned(),
            metadata_object(payload),
        )])),
    );
    Json::Object(fields)
}

// a request's own metadata rides nested in the standard payload, and promptlayer reads pl_tags and the rest flat
fn metadata_object(payload: &StandardLoggingPayload) -> Json {
    let mut fields =
        match serde_json::to_value(&payload.metadata).and_then(serde_json::from_value::<Json>) {
            Ok(Json::Object(fields)) => fields,
            Ok(value) => return value,
            Err(error) => {
                tracing::warn!(%error, "the promptlayer callback metadata could not be encoded");
                return Json::Object(IndexMap::new());
            }
        };
    if let Some(Json::Object(requester)) = payload.metadata.requester_metadata.as_ref() {
        for (name, value) in requester {
            fields.insert(name.clone(), value.clone());
        }
    }
    Json::Object(fields)
}

pub(super) fn utc_at(seconds: Decimal) -> Option<DateTime<Utc>> {
    seconds
        .checked_mul(Decimal::from(1_000_i64))
        .and_then(|milliseconds| milliseconds.round().to_i64())
        .and_then(DateTime::<Utc>::from_timestamp_millis)
}
