use betterlite_core::agents::json::AgentJson;
use betterlite_core::agents::provider::a2a::LiteLLMSendMessageResponse;
use futures_util::StreamExt as _;
use futures_util::stream::BoxStream;

use crate::completion_bridge::EventSource;
use crate::completion_bridge::completion::CompletionPort;
use crate::completion_bridge::dispatch::{A2AProviderDispatch, Buffered, Mode, Streamed, dispatch};
use crate::completion_bridge::params::{BridgeCall, ParamsError, optional_text};
use crate::logging_context::json_rpc_id_text;
use crate::provider_selector::{A2AProviderConfig, get_provider_config};

use super::call::{SendCall, StreamCall};
use super::deps::BridgeDeps;
use super::error::SendEntryError;
use super::params::bridge_params;
use super::response::response_from_dict;

pub(super) type StreamedBridge<'deps, C, P> = BoxStream<
    'deps,
    Result<
        AgentJson,
        SendEntryError<<C as CompletionPort>::Error, <P as A2AProviderDispatch>::Error>,
    >,
>;

pub(super) async fn send_via_completion_bridge<C, P, S>(
    call: &SendCall,
    provider: &str,
    bridge: &mut BridgeDeps<'_, C, P, S>,
) -> Result<LiteLLMSendMessageResponse, SendEntryError<C::Error, P::Error>>
where
    C: CompletionPort,
    P: A2AProviderDispatch,
    S: EventSource + Send,
{
    tracing::info!(provider = %provider, api_base = ?call.api_base, "A2A using completion bridge");
    let params = bridge_params(&call.request)?;
    let request_id = json_rpc_id_text(&call.request.id);
    let bridge_call = BridgeCall {
        request_id: &request_id,
        params: &params,
        litellm_params: &call.litellm_params,
        api_base: call.api_base.as_deref(),
        agent_extra_headers: call.agent_extra_headers.as_ref(),
    };
    let answer = dispatch::<Buffered, C, P, S>(
        &bridge_call,
        bridge.completion,
        bridge.providers,
        &mut *bridge.source,
    )
    .await?;
    response_from_dict(&answer, &request_id).map_err(SendEntryError::<C::Error, P::Error>::from)
}

pub(super) fn stream_via_completion_bridge<'deps, C, P, S>(
    call: StreamCall,
    completion: &'deps C,
    providers: &'deps P,
    mut source: &'deps mut S,
) -> StreamedBridge<'deps, C, P>
where
    C: CompletionPort,
    P: A2AProviderDispatch,
    S: EventSource + Send + 'deps,
{
    async_stream::stream! {
        let params = match bridge_params(&call.request) {
            Ok(params) => params,
            Err(error) => {
                yield Err(SendEntryError::<C::Error, P::Error>::from(error));
                return;
            }
        };
        let request_id = json_rpc_id_text(&call.request.id);
        let bridge_call = BridgeCall {
            request_id: &request_id,
            params: &params,
            litellm_params: &call.litellm_params,
            api_base: call.api_base.as_deref(),
            agent_extra_headers: call.agent_extra_headers.as_ref(),
        };
        // dispatch's Mode-generic future is not provably Send inside the stream (rust-lang/rust#100013)
        let mut events = match select_config(&bridge_call) {
            Ok(Some(config)) => {
                <Streamed as Mode<'_, C, P>>::provider(providers, config, &bridge_call).await
            }
            // direct's prefixed_model reads the same fields and reports the same ParamsError
            Ok(None) | Err(_) => {
                <Streamed as Mode<'_, C, P>>::direct(&bridge_call, completion, &mut source).await
            }
        };
        while let Some(event) = events.next().await {
            yield event.map_err(SendEntryError::<C::Error, P::Error>::Bridge);
        }
    }
    .boxed()
}

fn select_config(call: &BridgeCall<'_>) -> Result<Option<A2AProviderConfig>, ParamsError> {
    let provider = optional_text(call.litellm_params, "custom_llm_provider")?;
    let model = optional_text(call.litellm_params, "model")?;
    Ok(get_provider_config(provider, model))
}
