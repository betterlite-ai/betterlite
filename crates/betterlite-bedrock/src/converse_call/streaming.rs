use std::str::FromStr as _;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use betterlite_core::api::vector_store_http_write::service::ClientKind;
use betterlite_redact::secret::Redactor;
use http::header::AUTHORIZATION;
use http::{HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;

use crate::agentcore_stream::IdSource;
use crate::agentcore_stream::adapter::header_pairs;
use crate::bedrock_http_stream::converse::{ConverseCall as WireCall, make_sync_call, open_call};
use crate::bedrock_http_stream::stream::ChunkStream;
use crate::bedrock_http_stream::{PendingOpen, StreamDeps, invoke};
use crate::image_call::plan::LogContext;
use crate::invoke_events::DecoderClock;

use super::ConverseCall;
use super::error::ConverseCallError;
use super::prepare::CallPlan;
use super::transport::{ConverseClientFactory, StreamClient, SuppliedStream};

mod projection;

pub struct StreamParts<'parts, 'supplied> {
    pub route: ClientKind,
    pub timeout: Option<Duration>,
    pub supplied: Option<&'supplied SuppliedStream>,
    pub ids: Arc<dyn IdSource>,
    pub clock: DecoderClock,
    pub redactor: &'parts Redactor,
    pub now: SystemTime,
}

// The B005 CustomStreamWrapper shape: the iterator, the provider name and the headers the wrapper stores, with no frame read before the caller takes it.
pub struct WrappedStream<'deps> {
    pub completion_stream: ChunkStream<'deps>,
    pub model: String,
    pub custom_llm_provider: &'static str,
    pub response_headers: IndexMap<String, String>,
    pub stream_chunk_size: Option<u64>,
    pub redactor: &'deps Redactor,
}

pub struct PendingStream<'deps> {
    deps: StreamDeps<'deps>,
    pending: PendingOpen,
    route: ClientKind,
}

pub(super) async fn open_stream<'parts, Factory>(
    call: &ConverseCall<'_>,
    plan: &CallPlan,
    parts: &StreamParts<'parts, '_>,
    factory: &Factory,
) -> Result<PendingStream<'parts>, ConverseCallError>
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
    let wire = wire_call(call, plan)?;
    let pending = if parts.route == ClientKind::Asynchronous {
        invoke::open_call(&invoke_call(&wire), &deps).await?
    } else {
        open_call(&wire, &deps).await?
    };
    Ok(PendingStream {
        deps,
        pending,
        route: parts.route,
    })
}

pub(super) fn finish_stream<'deps>(
    call: &ConverseCall<'_>,
    plan: &CallPlan,
    pending: PendingStream<'deps>,
    log: &mut LogContext<'_, '_>,
) -> Result<WrappedStream<'deps>, ConverseCallError> {
    let wire = wire_call(call, plan)?;
    let opened = if pending.route == ClientKind::Asynchronous {
        invoke::make_call(&invoke_call(&wire), &pending.deps, pending.pending, log)?
    } else {
        make_sync_call(&wire, &pending.deps, pending.pending, log)?
    };
    Ok(WrappedStream {
        response_headers: header_pairs(&opened.headers).into_iter().collect(),
        completion_stream: opened.chunks,
        model: call.model.to_owned(),
        custom_llm_provider: "bedrock",
        stream_chunk_size: plan.stream_chunk_size,
        redactor: pending.deps.redactor,
    })
}

// The async streaming path accepts the call timeout but never hands it to make_call; a sync call builds its client with it, and a supplied client of the route keeps what it was built with.
fn client_for<Factory>(factory: &Factory, parts: &StreamParts<'_, '_>) -> StreamClient
where
    Factory: ConverseClientFactory,
{
    match (parts.route, parts.supplied) {
        (ClientKind::Asynchronous, Some(supplied)) if supplied.kind == ClientKind::Asynchronous => {
            supplied.client.clone()
        }
        (ClientKind::Sync, Some(supplied)) if supplied.kind == ClientKind::Sync => {
            supplied.client.clone()
        }
        (ClientKind::Asynchronous, _) => factory.stream(ClientKind::Asynchronous, None),
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

fn wire_call<'call>(
    call: &'call ConverseCall<'_>,
    plan: &'call CallPlan,
) -> Result<WireCall<'call>, ConverseCallError> {
    Ok(WireCall {
        model: call.model,
        api_base: &plan.url,
        headers: header_map(&plan.signed.headers)?,
        data: &plan.data,
        messages: &plan.messages,
        json_mode: plan.json_mode,
        fake_stream: plan.fake_stream,
    })
}

fn invoke_call<'call>(wire: &WireCall<'call>) -> invoke::InvokeCall<'call> {
    invoke::InvokeCall {
        model: wire.model,
        api_base: wire.api_base,
        headers: wire.headers.clone(),
        data: wire.data,
        signed_json_body: None,
        messages: wire.messages,
        fake_stream: wire.fake_stream,
        json_mode: wire.json_mode,
        provider: None,
        sync: false,
    }
}

#[path = "tests/streaming.rs"]
mod tests;
