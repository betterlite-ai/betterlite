use std::fmt;
use std::sync::Arc;

use betterlite_redis::batch::error::BatchError;

use crate::atomic_counter::response::{ResponseDecodeError, build_atomic_response};
use crate::limiter_records::{
    AtomicCounterMeta, CacheCounterValue, CounterReplyError, RateLimitResponse, ScriptReply,
    counter_values_from_reply,
};

mod tests;

pub(super) enum GroupResponse {
    Response {
        response: RateLimitResponse,
        values: Vec<CacheCounterValue>,
    },
    Failure(GroupFailure),
}

pub(super) enum GroupFailure {
    Batch(Arc<BatchError>),
    Reply(CounterReplyError),
    Decode(ResponseDecodeError),
}

impl fmt::Display for GroupFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Batch(error) => fmt::Display::fmt(error.as_ref(), formatter),
            Self::Reply(error) => fmt::Display::fmt(error, formatter),
            Self::Decode(error) => fmt::Display::fmt(error, formatter),
        }
    }
}

impl GroupResponse {
    pub(super) fn over_limit(&self) -> Option<&RateLimitResponse> {
        match self {
            Self::Response { response, .. } if response.overall_code == "OVER_LIMIT" => {
                Some(response)
            }
            Self::Response { .. } | Self::Failure(_) => None,
        }
    }

    pub(super) fn failure(&self) -> Option<&GroupFailure> {
        match self {
            Self::Failure(failure) => Some(failure),
            Self::Response { .. } => None,
        }
    }
}

pub(super) fn pipelined_group_response(
    reply: Result<ScriptReply, Arc<BatchError>>,
    meta: &[AtomicCounterMeta],
) -> GroupResponse {
    let reply = match reply {
        Ok(reply) => reply,
        Err(error) => return GroupResponse::Failure(GroupFailure::Batch(error)),
    };
    let values = match counter_values_from_reply(&reply) {
        Ok(values) => values,
        Err(error) => return GroupResponse::Failure(GroupFailure::Reply(error)),
    };
    match build_atomic_response(&values, meta) {
        Ok(response) => GroupResponse::Response { response, values },
        Err(error) => GroupResponse::Failure(GroupFailure::Decode(error)),
    }
}
