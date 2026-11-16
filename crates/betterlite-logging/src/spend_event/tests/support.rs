use betterlite_models::decisions::Json;

use crate::spend_event::build::build_spend_event;
use crate::spend_event::decode::decode_spend_event;
use crate::spend_event::error::SpendEventBuildError;
use crate::spend_event::event::{Mapping, SpendEvent};
use crate::spend_event::input::{BuildInput, CachePresence, ResponseObject, TypedAuths};
use crate::spend_event::time::EventTime;

pub(super) struct Options {
    pub(super) response: ResponseObject,
    pub(super) typed_auth: TypedAuths,
    pub(super) store_bodies: bool,
    pub(super) cache: CachePresence,
}

pub(super) fn options(store_bodies: bool, cache: CachePresence) -> Options {
    Options {
        response: ResponseObject::Absent,
        typed_auth: TypedAuths {
            metadata: None,
            litellm_metadata: None,
        },
        store_bodies,
        cache,
    }
}

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

pub(super) fn mapping(text: &str) -> Mapping {
    serde_json::from_str(text).unwrap()
}

pub(super) fn stamp(text: &str) -> EventTime {
    serde_json::from_str(&format!("\"{text}\"")).unwrap()
}

pub(super) fn build_line(kwargs: &str, call: &Options) -> Result<Vec<u8>, SpendEventBuildError> {
    let raw = mapping(kwargs);
    build_spend_event(&BuildInput {
        kwargs: &raw,
        typed_auth: &call.typed_auth,
        response: &call.response,
        start_time: stamp("2026-01-01T00:00:00Z"),
        end_time: stamp("2026-01-01T00:00:01Z"),
        store_bodies: call.store_bodies,
        cache: call.cache,
    })
}

pub(super) fn built(kwargs: &str, call: &Options) -> SpendEvent {
    decode_spend_event(&build_line(kwargs, call).unwrap()).unwrap()
}
