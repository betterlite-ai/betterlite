use std::collections::BTreeMap;

use betterlite_core::agents::json::AgentJson;
use betterlite_core::agents::provider::a2a::LiteLLMSendMessageResponse;
use betterlite_logging::logging_state::LoggingState;
use secrecy::SecretString;

use crate::card_resolver::card_url::get_agent_card_url;
use crate::client::retry::{SendTarget, execute_send_with_retry};
use crate::client::{A2AClient, A2AClientSettings, agent_card_path, create_a2a_client};
use crate::logging_context::{
    get_a2a_model_info, json_rpc_id_text, set_agent_id_on_logging_obj, set_litellm_params,
    set_usage_on_logging_obj,
};

use super::call::SendCall;
use super::deps::NativeDeps;
use super::error::SendCallError;
use super::headers::{client_headers, logging_trace_id};
use super::params::pricing_params;
use super::response::response_from_a2a;
use super::usage::usage_from;

pub(super) async fn create_client(
    api_base: Option<String>,
    extra_headers: BTreeMap<String, SecretString>,
    litellm_params: &BTreeMap<String, AgentJson>,
    streaming: bool,
    deps: &NativeDeps<'_>,
) -> Result<A2AClient, SendCallError> {
    let base_url = api_base.ok_or(SendCallError::MissingClient)?;
    let settings = A2AClientSettings {
        base_url,
        timeout: None,
        tls: deps.tls.clone(),
        extra_headers,
        streaming,
        relative_card_path: agent_card_path(litellm_params),
    };
    create_a2a_client(deps.http, settings, deps.environment)
        .await
        .map_err(SendCallError::Client)
}

// The agent card, its resolved URL, the base and the name both retry helpers receive
pub(super) fn send_target(
    client: &A2AClient,
    api_base: Option<&str>,
    agent_name: &str,
) -> SendTarget {
    let agent_card = Some(client.agent_card.clone());
    let card_url = agent_card
        .as_ref()
        .and_then(get_agent_card_url)
        .map(str::to_owned);
    SendTarget {
        agent_card,
        card_url,
        api_base: api_base.map(str::to_owned),
        agent_name: Some(agent_name.to_owned()),
    }
}

pub(super) async fn send_native(
    call: SendCall,
    mut logging: Option<&mut LoggingState>,
    deps: &NativeDeps<'_>,
) -> Result<LiteLLMSendMessageResponse, SendCallError> {
    let SendCall {
        a2a_client,
        request,
        api_base,
        litellm_params,
        agent_id,
        agent_extra_headers,
    } = call;
    let client = if let Some(existing) = a2a_client {
        existing
    } else {
        let trace_id = logging_trace_id(logging.as_deref(), deps.trace_ids);
        let headers = client_headers(&trace_id, agent_id.as_deref(), agent_extra_headers.as_ref());
        create_client(api_base.clone(), headers, &litellm_params, false, deps).await?
    };
    let agent_name = get_a2a_model_info(Some(&client.agent_card), logging.as_deref_mut());
    tracing::info!(
        request_id = %json_rpc_id_text(&request.id),
        agent = %agent_name,
        "A2A send_message request"
    );
    let mut target = send_target(&client, api_base.as_deref(), &agent_name);
    let a2a_response = execute_send_with_retry(client, &request, &mut target).await?;
    let response = response_from_a2a(&a2a_response, &request)?;
    let (prompt_tokens, completion_tokens) =
        usage_from(deps.counter, &request, &a2a_response).await?;
    set_usage_on_logging_obj(logging.as_deref_mut(), prompt_tokens, completion_tokens)?;
    set_litellm_params(logging.as_deref_mut(), &pricing_params(&litellm_params)?)?;
    set_agent_id_on_logging_obj(logging, agent_id.as_deref());
    Ok(response)
}
