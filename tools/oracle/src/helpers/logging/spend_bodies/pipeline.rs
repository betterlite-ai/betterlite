use betterlite_logging::spend_bodies::SpendStoragePolicy;
use betterlite_logging::spend_bodies::messages::storage_messages;
use betterlite_logging::spend_bodies::request::{RequestKwargs, storage_request_body};
use betterlite_logging::spend_bodies::session::SpendTimestamp;
use betterlite_logging::spend_bodies::support::NodeAllocator;
use betterlite_logging::spend_sanitize::SpendSanitizePolicy;
use betterlite_logging::spend_sanitize::value::SpendValue;
use chrono::{DateTime, FixedOffset, NaiveDateTime};
use serde_json::Value;

use super::support::{decode, invalid, mapping_of};

pub(super) struct Renderer<'nodes> {
    pub(super) policy: SpendStoragePolicy,
    pub(super) nodes: &'nodes mut NodeAllocator,
}

pub(super) fn request_text(
    render: &mut Renderer<'_>,
    litellm_params: &SpendValue,
    standard: Option<&SpendValue>,
    tools: Option<&SpendValue>,
    dynamic: Option<&SpendValue>,
) -> crate::helpers::HelperResult {
    let params = mapping_of(litellm_params)?;
    let kwargs = RequestKwargs {
        standard_logging_object: standard.and_then(|value| mapping_of(value).ok()),
        realtime_tools: tools,
        dynamic_params: dynamic,
    };
    Ok(Value::String(storage_request_body(
        params,
        Some(&kwargs),
        render.policy,
        render.nodes,
    )))
}

pub(super) fn messages_text(
    payload: Option<&SpendValue>,
    store_bodies: bool,
) -> crate::helpers::HelperResult {
    match payload {
        Some(stored) => Ok(Value::String(storage_messages(
            Some(mapping_of(stored)?),
            store_bodies,
        ))),
        None => Ok(Value::String("{}".to_owned())),
    }
}

pub(super) type OptionalDecoded = Result<Option<SpendValue>, Box<crate::helpers::HelperError>>;

pub(super) fn optional_decode(value: Option<&Value>, nodes: &mut NodeAllocator) -> OptionalDecoded {
    match value {
        Some(present) => Ok(Some(decode(present, nodes)?)),
        None => Ok(None),
    }
}

pub(super) const fn storage_policy_of(
    sanitize: SpendSanitizePolicy,
    turn_off_message_logging: bool,
) -> SpendStoragePolicy {
    SpendStoragePolicy {
        sanitize,
        turn_off_message_logging,
    }
}

pub(super) fn timestamp(text: &str) -> Result<SpendTimestamp, Box<crate::helpers::HelperError>> {
    if let Ok(aware) = DateTime::<FixedOffset>::parse_from_rfc3339(text) {
        return Ok(SpendTimestamp::Aware(aware));
    }
    if let Ok(naive) = NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f") {
        return Ok(SpendTimestamp::Naive(naive));
    }
    Err(invalid("timestamps must be ISO-8601 text"))
}
