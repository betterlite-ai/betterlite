use betterlite_models::decisions::Json;

use crate::spend_event::event::{LitellmParams, Mapping};
use crate::spend_event::input::{TypedAuth, TypedAuths};

const RESERVATION_KEY: &str = "user_api_key_budget_reservation";
const AUTH_KEY: &str = "user_api_key_auth";
const OTEL_SPAN_KEY: &str = "litellm_parent_otel_span";

fn object_field(mapping: &Mapping, key: &str) -> Option<Mapping> {
    let Some(Json::Object(value)) = mapping.get(key) else {
        return None;
    };
    Some(value.clone())
}

// The reservation sits where the request setup left it: a direct mapping, a typed auth object or a mapping auth
fn bucket_reservation(bucket: &Mapping, typed: Option<&TypedAuth>) -> Option<Mapping> {
    if let Some(direct) = object_field(bucket, RESERVATION_KEY) {
        return Some(direct);
    }
    if let Some(auth) = typed {
        return auth.budget_reservation.clone();
    }
    let auth = object_field(bucket, AUTH_KEY)?;
    object_field(&auth, "budget_reservation")
}

// litellm_metadata is read before metadata, and the first bucket that yields a reservation wins
#[must_use]
pub fn choose_reservation(params: &LitellmParams, typed: &TypedAuths) -> Option<Mapping> {
    let from_litellm_metadata = params
        .litellm_metadata
        .as_ref()
        .and_then(|bucket| bucket_reservation(bucket, typed.litellm_metadata.as_ref()));
    from_litellm_metadata.or_else(|| {
        params
            .metadata
            .as_ref()
            .and_then(|bucket| bucket_reservation(bucket, typed.metadata.as_ref()))
    })
}

// An absent bucket stays absent, and the chosen reservation is copied only into a bucket that exists
#[must_use]
pub fn event_bucket(bucket: Option<&Mapping>, reservation: Option<&Mapping>) -> Option<Mapping> {
    let source = bucket?;
    let mut kept: Mapping = source
        .iter()
        .filter(|(key, _)| key.as_str() != AUTH_KEY && key.as_str() != OTEL_SPAN_KEY)
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    if let Some(chosen) = reservation {
        kept.insert(RESERVATION_KEY.to_owned(), Json::Object(chosen.clone()));
    }
    Some(kept)
}
