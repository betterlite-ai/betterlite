use serde_json::Value;

use crate::spend_bodies::session::SpendTimestamp;
use crate::spend_event::build::build_spend_event;
use crate::spend_event::error::SpendEventBuildError;
use crate::spend_event::event::Mapping;
use crate::spend_event::input::{BuildInput, CachePresence, ResponseObject, TypedAuths};

pub struct Options {
    pub response: ResponseObject,
    pub typed_auth: TypedAuths,
    pub store_bodies: bool,
    pub cache: CachePresence,
}

pub fn options(store_bodies: bool, cache: CachePresence) -> Options {
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

pub fn mapping(text: &str) -> Mapping {
    serde_json::from_str(text).unwrap()
}

pub fn stamp(text: &str) -> SpendTimestamp {
    serde_json::from_value(Value::String(text.to_owned())).unwrap()
}

pub fn build_line(kwargs: &str, call: &Options) -> Result<Vec<u8>, SpendEventBuildError> {
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

pub fn built(kwargs: &str, call: &Options) -> Value {
    serde_json::from_slice(&build_line(kwargs, call).unwrap()).unwrap()
}
