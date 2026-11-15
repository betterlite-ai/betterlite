use betterlite_core::agents::json::AgentJson;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::correlation::CorrelationSetup;
use chrono::{DateTime, FixedOffset};
use futures_util::StreamExt as _;
use futures_util::stream::BoxStream;

use crate::client::A2AClient;
use crate::client::records::response::SendMessageResponse;
use crate::client::retry::{SendError, execute_stream_with_retry};
use crate::logging_context::{
    LoggingDeps, StreamingLoggingRequest, build_streaming_logging_obj, get_a2a_model_info,
    json_rpc_id_text, set_agent_id_on_logging_obj,
};
use crate::streaming_iterator::{
    A2AStreamingIterator, StreamSetup, StreamSuccessDispatch, StreamedChunk as _,
};

use super::call::StreamCall;
use super::deps::{NativeDeps, StreamingDeps};
use super::error::SendCallError;
use super::headers::{client_headers, stream_trace_id};
use super::native::{create_client, send_target};
use super::params::pricing_params;

// LiteLLM builds the streaming logging object only when the caller supplied none
fn build_logging<Clock, Dispatch>(
    call: &StreamCall,
    agent_name: &str,
    deps: &mut StreamingDeps<'_, Clock, Dispatch>,
) -> Result<LoggingState, SendCallError>
where
    Clock: FnMut() -> DateTime<FixedOffset> + Send,
    Dispatch: StreamSuccessDispatch,
{
    let pricing = pricing_params(&call.litellm_params)?;
    let generated_trace_id = (deps.native.trace_ids)();
    let details = StreamingLoggingRequest {
        request_id: &call.request.id,
        agent_name,
        agent_id: call.agent_id.as_deref(),
        litellm_params: Some(&pricing),
        metadata: call.metadata.as_ref(),
        proxy_server_request: call.proxy_server_request.as_ref(),
    };
    let logging = LoggingDeps {
        start_time: (deps.clock)(),
        correlation: CorrelationSetup {
            context: &mut *deps.correlation,
            settings: deps.correlation_settings,
        },
        redactor: deps.redactor,
        generated_trace_id: &generated_trace_id,
        clock: &mut *deps.monotonic,
    };
    build_streaming_logging_obj(&details, logging).map_err(SendCallError::Logging)
}

// LiteLLM's A2AStreamingIterator loop: every chunk reaches the caller as its wire form
fn wire_chunks<'deps, Dispatch, Clock>(
    mut iterator: A2AStreamingIterator<
        'deps,
        SendMessageResponse,
        BoxStream<'static, Result<SendMessageResponse, SendError>>,
        Dispatch,
        Clock,
    >,
) -> BoxStream<'deps, Result<AgentJson, SendCallError>>
where
    Dispatch: StreamSuccessDispatch + 'deps,
    Clock: FnMut() -> DateTime<FixedOffset> + Send + 'deps,
{
    async_stream::stream! {
        while let Some(item) = iterator.next_chunk::<SendError>().await {
            match item {
                Ok(chunk) => match chunk.wire() {
                    Ok(wire) => yield Ok(wire),
                    Err(source) => {
                        yield Err(SendCallError::Json { value: "streaming chunk", source });
                        return;
                    }
                },
                Err(error) => {
                    yield Err(SendCallError::Send(error));
                    return;
                }
            }
        }
    }
    .boxed()
}

// LiteLLM creates a streaming client only when the caller passed none, with the three-tier trace id
async fn native_client(
    call: &mut StreamCall,
    logging: Option<&LoggingState>,
    deps: &NativeDeps<'_>,
) -> Result<A2AClient, SendCallError> {
    if let Some(client) = call.a2a_client.take() {
        return Ok(client);
    }
    let trace_id = stream_trace_id(logging, &call.request.id, deps.trace_ids);
    let headers = client_headers(
        &trace_id,
        call.agent_id.as_deref(),
        call.agent_extra_headers.as_ref(),
    );
    create_client(
        call.api_base.clone(),
        headers,
        &call.litellm_params,
        true,
        deps,
    )
    .await
}

pub(super) async fn stream_native<'deps, Clock, Dispatch>(
    mut call: StreamCall,
    mut logging: Option<LoggingState>,
    mut deps: StreamingDeps<'deps, Clock, Dispatch>,
) -> Result<BoxStream<'deps, Result<AgentJson, SendCallError>>, SendCallError>
where
    Clock: FnMut() -> DateTime<FixedOffset> + Send + 'deps,
    Dispatch: StreamSuccessDispatch + 'deps,
{
    let client = native_client(&mut call, logging.as_ref(), &deps.native).await?;
    let agent_name = get_a2a_model_info(Some(&client.agent_card), logging.as_mut());
    let mut logging_state = match logging {
        Some(state) => state,
        None => build_logging(&call, &agent_name, &mut deps)?,
    };
    let target = send_target(&client, call.api_base.as_deref(), &agent_name);
    tracing::info!(
        request_id = %json_rpc_id_text(&call.request.id),
        agent = %agent_name,
        "A2A send_message_streaming request"
    );
    let source = execute_stream_with_retry(client, call.request.clone(), target);
    set_agent_id_on_logging_obj(Some(&mut logging_state), call.agent_id.as_deref());
    let setup = StreamSetup {
        counter: deps.native.counter,
        max_retained_bytes: deps.max_retained_bytes,
        dispatch: deps.dispatch,
        clock: deps.clock,
    };
    let iterator = A2AStreamingIterator::new(source, call.request, logging_state, setup);
    Ok(wire_chunks(iterator))
}
