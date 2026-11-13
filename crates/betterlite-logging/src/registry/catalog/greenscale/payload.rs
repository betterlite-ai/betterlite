use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;

use crate::fields::payload::StandardLoggingPayload;

// greenscale reads only the model and the project metadata out of the callback kwargs
pub(super) fn callback_kwargs(payload: &StandardLoggingPayload) -> Json {
    let mut fields = IndexMap::new();
    fields.insert("model".to_owned(), Json::Text(payload.model.clone()));
    fields.insert(
        "litellm_params".to_owned(),
        Json::Object(IndexMap::from([(
            "metadata".to_owned(),
            metadata_object(payload),
        )])),
    );
    Json::Object(fields)
}

// a request's own metadata rides nested in the standard payload, and greenscale reads its prefixed keys flat
fn metadata_object(payload: &StandardLoggingPayload) -> Json {
    let Some(Json::Object(requester)) = payload.metadata.requester_metadata.as_ref() else {
        return Json::Object(IndexMap::new());
    };
    Json::Object(
        requester
            .iter()
            .filter(|(name, _value)| name.starts_with("greenscale"))
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
    )
}

pub(super) fn utc_at(seconds: Decimal) -> Option<DateTime<Utc>> {
    seconds
        .checked_mul(Decimal::from(1_000_i64))
        .and_then(|milliseconds| milliseconds.round().to_i64())
        .and_then(DateTime::<Utc>::from_timestamp_millis)
}
