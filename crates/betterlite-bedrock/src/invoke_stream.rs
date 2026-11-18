use std::convert::Infallible;
use std::sync::Arc;

use betterlite_logging::python_json;
use betterlite_models::controls::control_options::ControlOptions;
use betterlite_models::decisions::Json;
use http::HeaderMap;
use indexmap::IndexMap;

use crate::agentcore_stream::transport::{HandlerKind, StreamHandler};
use crate::aws_endpoints::provider::{BedrockInvokeProvider, invoke_provider};
use crate::bedrock_http_stream::StreamDeps;
use crate::bedrock_http_stream::error::HttpStreamError;
use crate::bedrock_http_stream::invoke::{InvokeCall, make_call, open_call};
use crate::bedrock_http_stream::stream::{ChunkStream, OpenedStream};
use crate::image_call::plan::LogContext;
use crate::invoke_events::DecoderClock;

mod chunking;
mod tests;

pub const CUSTOM_LLM_PROVIDER: &str = "bedrock";
pub const HAS_CUSTOM_STREAM_WRAPPER: bool = true;
pub const SUPPORTS_STREAM_PARAM_IN_BODY: bool = false;

// What either stream wrapper passes to make_call/make_sync_call, plus the control option B691's call has no slot for.
pub struct InvokeStreamCall<'call> {
    pub model: &'call str,
    pub api_base: &'call str,
    pub headers: HeaderMap,
    pub data: &'call IndexMap<String, Json>,
    pub messages: &'call Json,
    pub json_mode: Option<bool>,
    pub signed_json_body: Option<&'call [u8]>,
    pub control: ControlOptions,
}

pub struct StreamTransport<'call> {
    pub model: &'call str,
    pub api_base: &'call str,
    pub headers: HeaderMap,
    pub data: String,
    pub signed_json_body: Option<&'call [u8]>,
    pub messages: &'call Json,
    pub fake_stream: bool,
    pub json_mode: Option<bool>,
    pub provider: Option<&'static str>,
    pub sync: bool,
    pub stream_chunk_size: Option<u64>,
}

// The part of B004's CustomStreamWrapper the composition sets: the stream B691 opened and the identity around it.
pub struct WrappedInvokeStream<'deps> {
    pub completion_stream: ChunkStream<'deps>,
    pub model: String,
    pub custom_llm_provider: &'static str,
    pub response_headers: HeaderMap,
}

#[must_use]
pub fn stream_transport<'call>(
    call: &'call InvokeStreamCall<'call>,
    sync: bool,
) -> StreamTransport<'call> {
    StreamTransport {
        model: call.model,
        api_base: call.api_base,
        headers: call.headers.clone(),
        data: python_json::dumps_object(call.data),
        // get_async_custom_stream_wrapper never passes signed_json_body on; the sync one always does.
        signed_json_body: if sync { call.signed_json_body } else { None },
        messages: call.messages,
        // fake_stream=True if "ai21" in api_base: the base alone decides, and the test is case-sensitive.
        fake_stream: call.api_base.contains("ai21"),
        json_mode: call.json_mode,
        provider: invoke_provider(call.model).map(BedrockInvokeProvider::name),
        sync,
        stream_chunk_size: call.control.stream_chunk_size,
    }
}

// Both LiteLLM wrappers end in this composition; the transport's sync flag is what it takes from either one.
pub async fn custom_stream_wrapper<'deps>(
    transport: &StreamTransport<'_>,
    deps: &StreamDeps<'deps>,
    log: &mut LogContext<'_, '_>,
) -> Result<WrappedInvokeStream<'deps>, HttpStreamError<Infallible>> {
    let call = InvokeCall {
        model: transport.model,
        api_base: transport.api_base,
        headers: transport.headers.clone(),
        data: &transport.data,
        signed_json_body: transport.signed_json_body,
        messages: transport.messages,
        fake_stream: transport.fake_stream,
        json_mode: transport.json_mode,
        provider: transport.provider,
        sync: transport.sync,
    };
    let synchronous = sync_deps(deps);
    let effective = if transport.sync { &synchronous } else { deps };
    let pending = open_call(&call, effective).await?;
    let opened = make_call(
        &call,
        effective,
        chunking::apply(pending, transport.stream_chunk_size),
        log,
    )?;
    let OpenedStream { chunks, headers } = opened;
    Ok(WrappedInvokeStream {
        completion_stream: chunks,
        model: transport.model.to_owned(),
        custom_llm_provider: CUSTOM_LLM_PROVIDER,
        response_headers: headers,
    })
}

// _get_httpx_client(params={}) replaces a missing or async client; only a sync caller client is kept.
#[must_use]
pub fn sync_client(deps: &StreamDeps<'_>) -> Arc<dyn StreamHandler> {
    match &deps.supplied {
        Some(handler) if handler.kind() == HandlerKind::Sync => Arc::clone(handler),
        Some(_) | None => Arc::clone(&deps.default),
    }
}

fn sync_deps<'deps>(deps: &StreamDeps<'deps>) -> StreamDeps<'deps> {
    StreamDeps {
        supplied: Some(sync_client(deps)),
        default: Arc::clone(&deps.default),
        ids: Arc::clone(&deps.ids),
        clock: DecoderClock {
            next_id: deps.clock.next_id,
            now: deps.clock.now,
        },
        redactor: deps.redactor,
        timeout: deps.timeout,
        now: deps.now,
    }
}
