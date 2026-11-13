use std::error::Error;
use std::future::Future;
use std::pin::Pin;

use betterlite_models::call_helpers::metadata::{
    CallParams, InternalMetadata, MetadataMap, litellm_metadata_from_params,
};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

pub const BUDGET_RESERVATION_KEY: &str = "user_api_key_budget_reservation";

pub type ReleaseFuture<'release, Failure> =
    Pin<Box<dyn Future<Output = Result<(), Failure>> + Send + 'release>>;

// the server wires this over the shared budget services; the logging object only finds the reservation
pub trait ReservationRelease: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn release<'release>(
        &'release self,
        reservation: &'release MetadataMap,
    ) -> ReleaseFuture<'release, Self::Error>;
}

#[must_use]
pub fn reservation_from_params(params: &IndexMap<String, Json>) -> Option<MetadataMap> {
    let metadata = litellm_metadata_from_params(Some(&call_params(params)));
    metadata
        .get(BUDGET_RESERVATION_KEY)
        .and_then(object)
        .cloned()
}

fn call_params(params: &IndexMap<String, Json>) -> CallParams {
    CallParams {
        metadata: params.get("metadata").and_then(object).cloned(),
        litellm_metadata: params
            .get("litellm_metadata")
            .and_then(object)
            .map(|entries| InternalMetadata {
                entries: entries.clone(),
            }),
    }
}

const fn object(value: &Json) -> Option<&IndexMap<String, Json>> {
    match value {
        Json::Object(fields) => Some(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}
