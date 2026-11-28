use std::sync::Arc;

use betterlite_http::error::HttpError;
use betterlite_openai::passthrough::transformation::types::PassthroughTarget;
use bytes::Bytes;
use futures_util::StreamExt as _;
use futures_util::stream;
use http::HeaderMap;
use indexmap::IndexMap;
use tokio_util::task::TaskTracker;

use super::{Failure, RelayContext, RelayStatus};
use crate::api::passthrough_call::config::PassthroughCallConfig;
use crate::api::passthrough_call::logging::RelayLogging;
use crate::api::passthrough_call::wire::header_text;
use crate::api::passthrough_call::{PassthroughBody, PassthroughRelay, PassthroughReply};
use crate::passthrough_stream::async_lifecycle::AsyncPassthroughStream;
use crate::passthrough_stream::async_passthrough_request::FullReply;
use crate::passthrough_stream::error::UpstreamStatus;
use crate::passthrough_stream::response::StreamBody;
use crate::passthrough_stream::spend_collection::{OwnedProviderSpend, SpendCollection};

// raise_for_status fails only 4xx and 5xx, so a redirect is still the answer httpx hands back
pub(super) fn answer(
    status: UpstreamStatus,
    streaming: bool,
    context: &RelayContext<'_>,
    logging: Arc<RelayLogging>,
) -> Result<PassthroughReply, Failure> {
    let headers = header_pairs(&status.headers);
    if status.status_code >= 400 {
        return Err(Failure::Upstream(Box::new(RelayStatus {
            status: status.status_code,
            headers,
            body: Bytes::from(status.body),
        })));
    }
    if streaming {
        return Ok(synthetic_stream(status, context, logging));
    }
    Ok(PassthroughReply {
        status: status.status_code,
        headers,
        body: PassthroughBody::Full(Box::new(Bytes::from(status.body))),
    })
}

// The relay already read the redirect body, so the stream hands those bytes back as its only chunk
fn synthetic_stream(
    status: UpstreamStatus,
    context: &RelayContext<'_>,
    logging: Arc<RelayLogging>,
) -> PassthroughReply {
    let headers = header_pairs(&status.headers);
    let target = PassthroughTarget {
        model: context.model,
        custom_llm_provider: context.provider,
        endpoint: context.endpoint,
    };
    let stream: PassthroughRelay = AsyncPassthroughStream {
        initialized: true,
        status_code: status.status_code,
        headers: status.headers,
        pending: None,
        source: Some(Box::new(StreamBody {
            chunks: stream::iter(vec![Ok::<Vec<u8>, HttpError>(status.body)]).boxed(),
            body_limit: context.body_limit,
        })),
        spend: SpendCollection {
            sink: Some(OwnedProviderSpend::new(
                PassthroughCallConfig {
                    tag: context.tag,
                    environment: Arc::clone(&context.environment),
                    aws: Arc::clone(&context.aws),
                },
                target,
            )),
            chunk_count: 0,
            failed: false,
        },
        logging,
        background_tasks: TaskTracker::new(),
        flush_scheduled: false,
        hidden_params: IndexMap::new(),
    };
    PassthroughReply {
        status: stream.status_code,
        headers,
        body: PassthroughBody::Stream(Box::new(stream)),
    }
}

pub(super) fn full(reply: FullReply) -> PassthroughReply {
    PassthroughReply {
        status: reply.status_code,
        headers: header_pairs(&reply.headers),
        body: PassthroughBody::Full(Box::new(Bytes::from(reply.body))),
    }
}

pub(super) fn stream(stream: Box<PassthroughRelay>) -> Result<PassthroughReply, Failure> {
    let status = stream
        .status_code()
        .map_err(|error| super::relay_fault(&error))?;
    let headers = stream
        .headers()
        .map_err(|error| super::relay_fault(&error))?;
    Ok(PassthroughReply {
        status,
        headers: header_pairs(headers),
        body: PassthroughBody::Stream(stream),
    })
}

pub(super) fn header_pairs(headers: &HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| (name.as_str().to_owned(), header_text(value)))
        .collect()
}
