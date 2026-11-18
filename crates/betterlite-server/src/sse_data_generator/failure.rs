use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_error::envelope::{ErrorObject, ErrorResponse, WireField};
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};

use super::types::{FailureParam, StreamFailure};
use crate::stream_chunks::frames::sse_data_frame;

pub(super) fn unix_timestamp() -> i64 {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    i64::try_from(seconds).unwrap_or(i64::MAX)
}

pub(super) fn failure_frame(failure: &StreamFailure) -> String {
    let exception = ProxyException::new(ProxyExceptionInit {
        message: failure.message.clone(),
        error_type: match &failure.error_type {
            Some(text) => text.clone(),
            None => "None".to_owned(),
        },
        param: match &failure.param {
            FailureParam::Text(text) => Some(text.clone()),
            FailureParam::Missing | FailureParam::Null => None,
        },
        code: ExceptionCode::Status(i64::from(failure.status_code.unwrap_or(500))),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    });
    let error = ErrorObject {
        message: exception.message,
        error_type: WireField::Text(exception.error_type),
        param: wire_param(&failure.param),
        code: WireField::Text(exception.code),
    };
    // ErrorResponse always serializes; an impossible encoding failure must not lose the stream
    sse_data_frame(&serde_json::to_string(&ErrorResponse { error }).unwrap_or_default())
}

fn wire_param(param: &FailureParam) -> WireField {
    match param {
        FailureParam::Missing => WireField::MISSING,
        FailureParam::Null => WireField::Null,
        FailureParam::Text(text) => WireField::Text(text.clone()),
    }
}
