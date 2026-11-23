use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_a2a::client::A2AClient;
use betterlite_a2a::client::A2AClientSettings;
use betterlite_a2a::client::records::compat::SendMessageRequest;
use betterlite_config::tls::TlsPolicy;
use betterlite_core::agents::json::AgentJson;
use betterlite_core::agents::provider::a2a::LiteLLMSendMessageResponse;
use betterlite_models::decisions::Json;
use secrecy::SecretString;
use serde::Deserialize;

use super::error::A2aCallError;

// The settings JSON create_client and get_agent_card stand for, one key per LiteLLM argument.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Settings {
    pub(crate) base_url: String,
    pub(crate) timeout: Option<f64>,
    pub(crate) extra_headers: Option<BTreeMap<String, SecretString>>,
    pub(crate) streaming: bool,
    pub(crate) relative_card_path: Option<String>,
}

// The call JSON both send entry points take, one key per LiteLLM asend_message argument.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Call {
    pub(crate) request: Option<SendMessageRequest>,
    pub(crate) api_base: Option<String>,
    pub(crate) litellm_params: Option<BTreeMap<String, AgentJson>>,
    pub(crate) agent_id: Option<String>,
    pub(crate) agent_extra_headers: Option<BTreeMap<String, String>>,
    pub(crate) litellm_trace_id: Option<String>,
    pub(crate) metadata: Option<Json>,
}

// The call's fields once the request is known to be there, before either send shapes them its own way.
pub(crate) struct Resolved {
    pub(crate) trace_id: Option<String>,
    pub(crate) client: Option<A2AClient>,
    pub(crate) request: SendMessageRequest,
    pub(crate) api_base: Option<String>,
    pub(crate) litellm_params: BTreeMap<String, AgentJson>,
    pub(crate) agent_id: Option<String>,
    pub(crate) agent_extra_headers: Option<BTreeMap<String, String>>,
    pub(crate) metadata: Option<Json>,
}

impl Settings {
    pub(crate) fn into_settings(self, tls: TlsPolicy) -> Result<A2AClientSettings, A2aCallError> {
        let timeout = match self.timeout {
            Some(seconds) => Some(
                Duration::try_from_secs_f64(seconds)
                    .map_err(|_source| A2aCallError::Timeout { field: "timeout" })?,
            ),
            None => None,
        };
        Ok(A2AClientSettings {
            base_url: self.base_url,
            timeout,
            tls,
            extra_headers: self.extra_headers.unwrap_or_default(),
            streaming: self.streaming,
            relative_card_path: self.relative_card_path,
        })
    }
}

impl Call {
    // LiteLLM states a missing request differently once litellm_params selects the completion bridge.
    fn missing_request(litellm_params: &BTreeMap<String, AgentJson>) -> A2aCallError {
        let bridge = litellm_params
            .get("custom_llm_provider")
            .and_then(AgentJson::as_str)
            .is_some_and(|provider| !provider.is_empty());
        if bridge {
            return A2aCallError::MissingBridgeRequest;
        }
        A2aCallError::MissingRequest
    }

    pub(crate) fn resolve(self, client: Option<A2AClient>) -> Result<Resolved, A2aCallError> {
        let litellm_params = self.litellm_params.unwrap_or_default();
        let Some(request) = self.request else {
            return Err(Self::missing_request(&litellm_params));
        };
        Ok(Resolved {
            trace_id: self.litellm_trace_id,
            client,
            request,
            api_base: self.api_base,
            litellm_params,
            agent_id: self.agent_id,
            agent_extra_headers: self.agent_extra_headers,
            metadata: self.metadata,
        })
    }
}

// The one boundary every native method crosses: the text into the shape that method takes.
pub(crate) fn parse<'text, T>(text: &'text str) -> Result<T, A2aCallError>
where
    T: Deserialize<'text>,
{
    match serde_json::from_str(text) {
        Ok(value) => Ok(value),
        Err(source) => Err(A2aCallError::Json { source }),
    }
}

// LiteLLMSendMessageResponse skips _hidden_params on the way out, so the text carries it back by hand.
pub(crate) fn response_text(response: &LiteLLMSendMessageResponse) -> Result<String, A2aCallError> {
    let mut value =
        serde_json::to_value(response).map_err(|source| A2aCallError::Encode { source })?;
    let hidden = serde_json::to_value(&response.hidden_params)
        .map_err(|source| A2aCallError::Encode { source })?;
    if let Some(object) = value.as_object_mut() {
        object.insert(String::from("_hidden_params"), hidden);
    }
    serde_json::to_string(&value).map_err(|source| A2aCallError::Encode { source })
}

mod tests;
