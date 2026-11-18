use std::str::FromStr as _;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use betterlite_redact::secret::Redactor;
use http::header::AUTHORIZATION;
use http::{HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;

use crate::agentcore_stream::IdSource;
use crate::agentcore_stream::adapter::header_pairs;
use crate::bedrock_http_stream::StreamDeps;
use crate::bedrock_http_stream::converse::{ConverseCall as WireCall, make_sync_call, open_call};
use crate::bedrock_http_stream::stream::{ChunkStream, OpenedStream};
use crate::image_call::plan::LogContext;
use crate::invoke_events::DecoderClock;
use crate::rerank::transport::ClientKind;

use super::ConverseCall;
use super::error::ConverseCallError;
use super::prepare::CallPlan;
use super::transport::{ConverseClientFactory, StreamClient, SuppliedStream};

pub struct StreamParts<'parts, 'supplied> {
    pub route: ClientKind,
    pub timeout: Option<Duration>,
    pub supplied: Option<&'supplied SuppliedStream>,
    pub ids: Arc<dyn IdSource>,
    pub clock: DecoderClock,
    pub redactor: &'parts Redactor,
    pub now: SystemTime,
}

// The B005 CustomStreamWrapper shape: the iterator, the provider name and the headers the wrapper
// stores, with no frame read before the caller takes it.
pub struct WrappedStream<'deps> {
    pub completion_stream: ChunkStream<'deps>,
    pub model: String,
    pub custom_llm_provider: &'static str,
    pub response_headers: IndexMap<String, String>,
    pub stream_chunk_size: Option<u64>,
}

pub(crate) async fn open_stream<'parts, 'supplied, Factory>(
    call: &ConverseCall<'_>,
    plan: &CallPlan,
    parts: &StreamParts<'parts, 'supplied>,
    factory: &Factory,
    log: &mut LogContext<'_, '_>,
) -> Result<WrappedStream<'parts>, ConverseCallError>
where
    Factory: ConverseClientFactory,
{
    let client = client_for(factory, parts);
    let deps = StreamDeps {
        supplied: Some(Arc::clone(&client.handler)),
        default: Arc::clone(&client.handler),
        ids: Arc::clone(&parts.ids),
        clock: parts.clock,
        redactor: parts.redactor,
        timeout: client.timeout,
        now: parts.now,
    };
    let wire = WireCall {
        model: call.model,
        api_base: &plan.url,
        headers: header_map(&plan.signed.headers)?,
        data: &plan.data,
        messages: &plan.messages,
        json_mode: plan.json_mode,
        fake_stream: plan.fake_stream,
    };
    let pending = open_call(&wire, &deps).await?;
    let opened = make_sync_call(&wire, &deps, pending, log)?;
    Ok(wrap(opened, call, plan))
}

// The async streaming path accepts the call timeout but never hands it to make_call; a sync call
// builds its client with it, and a supplied client of the route keeps what it was built with.
fn client_for<Factory>(factory: &Factory, parts: &StreamParts<'_, '_>) -> StreamClient
where
    Factory: ConverseClientFactory,
{
    match (parts.route, parts.supplied) {
        (ClientKind::Async, Some(supplied)) if supplied.kind == ClientKind::Async => {
            supplied.client.clone()
        }
        (ClientKind::Sync, Some(supplied)) if supplied.kind == ClientKind::Sync => {
            supplied.client.clone()
        }
        (ClientKind::Async, _) => factory.stream(ClientKind::Async, None),
        (ClientKind::Sync, _) => factory.stream(ClientKind::Sync, parts.timeout),
    }
}

fn header_map(pairs: &[(String, String)]) -> Result<HeaderMap, ConverseCallError> {
    let mut map = HeaderMap::with_capacity(pairs.len());
    for (name, value) in pairs {
        let invalid = |source: http::Error| ConverseCallError::HeaderValue {
            name: name.clone(),
            source,
        };
        let header_name =
            HeaderName::from_str(name).map_err(|source| invalid(http::Error::from(source)))?;
        let mut header_value =
            HeaderValue::from_str(value).map_err(|source| invalid(http::Error::from(source)))?;
        if header_name == AUTHORIZATION {
            header_value.set_sensitive(true);
        }
        map.insert(header_name, header_value);
    }
    Ok(map)
}

fn wrap<'deps>(
    opened: OpenedStream<'deps>,
    call: &ConverseCall<'_>,
    plan: &CallPlan,
) -> WrappedStream<'deps> {
    WrappedStream {
        response_headers: header_pairs(&opened.headers).into_iter().collect(),
        completion_stream: opened.chunks,
        model: call.model.to_owned(),
        custom_llm_provider: "bedrock",
        stream_chunk_size: plan.stream_chunk_size,
    }
}
