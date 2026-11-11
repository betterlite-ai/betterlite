use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;

use crate::fields::payload::StandardLoggingPayload;

// the standard logging payload stands in for LiteLLM's model_call_details; only the keys athina reads are built
pub(super) fn callback_kwargs(payload: &StandardLoggingPayload, stream: bool) -> Json {
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
    fields.insert("stream".to_owned(), Json::Bool(stream));
    if stream && let Some(response) = payload.response.as_ref() {
        fields.insert("complete_streaming_response".to_owned(), response.clone());
    }
    Json::Object(fields)
}

// a request's own metadata rides nested in the standard payload, and athina reads those keys from the flat map
fn metadata_object(payload: &StandardLoggingPayload) -> Json {
    let mut fields =
        match serde_json::to_value(&payload.metadata).and_then(serde_json::from_value::<Json>) {
            Ok(Json::Object(fields)) => fields,
            Ok(value) => return value,
            Err(error) => {
                tracing::warn!(%error, "the athina callback metadata could not be encoded");
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
