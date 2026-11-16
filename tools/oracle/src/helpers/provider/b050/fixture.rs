use std::collections::BTreeMap;
use std::fmt::Display;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use async_trait::async_trait;
use betterlite_foundry::credentials::error::CredentialsError;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::Message;
use betterlite_provider::a2a_chat::auth::EntraResolver;
use betterlite_provider::a2a_chat::registry::{A2aAgentLookup, RegisteredA2aAgent};
use betterlite_provider::a2a_chat::reply as a2a_reply;
use betterlite_provider::a2a_chat::request as a2a_request;
use betterlite_provider::agent_platform_dispatch::call::CompletionCall;
use betterlite_provider::agent_platform_dispatch::handler::{ChatCompletionHandler, HandlerError};
use betterlite_provider::langflow_chat::call as langflow_call;
use betterlite_provider::langflow_chat::error::LangFlowError;
use betterlite_provider::langflow_chat::reply as langflow_reply;
use betterlite_provider::langgraph_chat::config as langgraph_config;
use betterlite_provider::langgraph_chat::records::LangGraphRole;
use betterlite_provider::langgraph_chat::reply as langgraph_reply;
use betterlite_provider::langgraph_chat::request as langgraph_request;
use betterlite_provider::ollama_chat::ids::RandomIds;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use betterlite_tokenizer::count::error::CountError;
use secrecy::SecretString;
use serde::Serialize;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use super::script::{AgentCase, Platform, UpstreamCase};

const KEPT_HEADERS: [&str; 4] = ["authorization", "content-type", "x-api-key", "x-card"];

struct NoTraffic;

impl OidcHttp for NoTraffic {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: String::from("the dispatch helper reads no OIDC document"),
        })
    }
}

impl AzureOidcTokens for NoTraffic {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Err(AzureOidcError::Provider {
            message: String::from("the dispatch helper mints no Azure token"),
        })
    }
}

impl OidcClock for NoTraffic {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_seconds(&self) -> i64 {
        0
    }
}

pub(super) fn secret_lookup() -> SecretLookup {
    let transport = Arc::new(NoTraffic);
    SecretLookup {
        manager: None,
        oidc_http: Arc::clone(&transport),
        azure_tokens: Arc::clone(&transport),
        clock: transport,
        oidc_cache: OidcTokenCache::default(),
    }
}

pub(super) struct NoEntra;

impl EntraResolver for NoEntra {
    fn token(&self, _params: &Object) -> Result<SecretString, CredentialsError> {
        Err(CredentialsError::ProviderFailed {
            message: String::from("the dispatch helper holds no Azure AD tenant"),
        })
    }

    fn auth_header(&self, _params: &Object) -> Result<BTreeMap<String, String>, CredentialsError> {
        Err(CredentialsError::ProviderFailed {
            message: String::from("the dispatch helper holds no hop header"),
        })
    }
}

pub(super) struct Registry {
    agents: BTreeMap<String, RegisteredA2aAgent>,
}

impl Registry {
    pub(super) fn new(cases: &[AgentCase]) -> Self {
        let mut agents = BTreeMap::new();
        for case in cases {
            let agent = RegisteredA2aAgent {
                agent_card_params: case.agent_card_params.clone(),
                litellm_params: case.litellm_params.clone(),
            };
            agents.insert(case.agent_name.clone(), agent);
        }
        Self { agents }
    }
}

impl A2aAgentLookup for Registry {
    fn agent_by_name(&self, name: &str) -> Option<RegisteredA2aAgent> {
        self.agents.get(name).cloned()
    }
}

// the reply carries no usage and no ids into the compared answer, so every count is one
pub(super) struct Meter;

#[async_trait]
impl a2a_reply::TokenCounter for Meter {
    async fn count_messages(&self, _model: &str, _messages: &[Message]) -> Result<u64, CountError> {
        Ok(1)
    }

    async fn count_response_tokens(&self, _model: &str, _text: &str) -> Result<u64, CountError> {
        Ok(1)
    }
}

#[async_trait]
impl langgraph_reply::TokenCounter for Meter {
    async fn count_messages(&self, _model: &str, _messages: &[Message]) -> Result<u64, CountError> {
        Ok(1)
    }

    async fn count_response_tokens(&self, _model: &str, _text: &str) -> Result<u64, CountError> {
        Ok(1)
    }
}

#[async_trait]
impl langflow_reply::TokenCounter for Meter {
    async fn count_messages(&self, _model: &str, _messages: &[Message]) -> Result<u64, CountError> {
        Ok(1)
    }

    async fn count_response_tokens(&self, _model: &str, _text: &str) -> Result<u64, CountError> {
        Ok(1)
    }
}

pub(super) struct Handler {
    platform: Platform,
    upstream: UpstreamCase,
    runtime: Runtime,
    requests: Mutex<Vec<Value>>,
}

impl Handler {
    pub(super) fn new(
        platform: Platform,
        upstream: &UpstreamCase,
    ) -> Result<Self, Box<HelperError>> {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .or_config_error()?;
        Ok(Self {
            platform,
            upstream: upstream.clone(),
            runtime,
            requests: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn requests(&self) -> Vec<Value> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn answer_a2a(
        &self,
        call: &CompletionCall,
        status: u16,
        body: &[u8],
    ) -> Result<Value, HandlerError> {
        let headers = BTreeMap::new();
        let reply = self
            .runtime
            .block_on(a2a_reply::transform_response(
                &a2a_reply::ReplyInput {
                    model: &call.model,
                    status,
                    headers: &headers,
                    body,
                    messages: &call.messages,
                },
                &RandomIds,
                &Meter,
            ))
            .map_err(|error| HandlerError {
                status_code: Some(error.status_code),
                message: error.message,
            })?;
        let Some(choice) = reply.choices.first() else {
            return Err(HandlerError {
                status_code: None,
                message: String::from("the A2A reply holds no choice"),
            });
        };
        Ok(json!({
            "model": reply.model,
            "content": choice.message.content,
            "role": choice.message.role,
            "finish_reason": choice.finish_reason,
        }))
    }

    fn answer_langgraph(
        &self,
        call: &CompletionCall,
        status: u16,
        body: &[u8],
    ) -> Result<Value, HandlerError> {
        let reply = self
            .runtime
            .block_on(langgraph_reply::transform_response(
                &langgraph_reply::ReplyInput {
                    model: &call.model,
                    status,
                    body,
                    messages: &call.messages,
                },
                &Meter,
            ))
            .map_err(|error| HandlerError {
                status_code: Some(error.status_code),
                message: error.message,
            })?;
        let Some(choice) = reply.choices.first() else {
            return Err(HandlerError {
                status_code: None,
                message: String::from("the LangGraph reply holds no choice"),
            });
        };
        Ok(json!({
            "model": reply.model,
            "content": choice.content,
            "role": role_text(choice.role),
            "finish_reason": choice.finish_reason,
        }))
    }

    fn answer_langflow(
        &self,
        call: &CompletionCall,
        status: u16,
        body: &[u8],
    ) -> Result<Value, HandlerError> {
        let reply = self
            .runtime
            .block_on(langflow_reply::transform_response(
                &langflow_reply::ReplyInput {
                    model: &call.model,
                    status,
                    body,
                    messages: &call.messages,
                },
                &Meter,
            ))
            .map_err(langflow_failure)?;
        Ok(json!({
            "model": reply.model,
            "content": reply.choice.message.content,
            "role": reply.choice.message.role,
            "finish_reason": reply.choice.finish_reason,
        }))
    }
}

impl ChatCompletionHandler for Handler {
    type Reply = Value;

    fn complete(&self, call: &CompletionCall) -> Result<Value, HandlerError> {
        let request = match self.platform {
            Platform::A2a => a2a_request_value(call)?,
            Platform::Langgraph => langgraph_request_value(call)?,
            Platform::Langflow => langflow_request_value(call)?,
        };
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        let status = self.upstream.status;
        if status >= 400 {
            // LiteLLM's shared handler turns a 4xx or 5xx into the provider error before any reply transform
            return Err(HandlerError {
                status_code: Some(status),
                message: self.upstream.body.clone(),
            });
        }
        if call.stream == Some(true) {
            // a streaming call hands back a stream rather than a reply, and this harness carries no chunks
            return Ok(Value::Null);
        }
        let body = self.upstream.body.as_bytes();
        match self.platform {
            Platform::A2a => self.answer_a2a(call, status, body),
            Platform::Langgraph => self.answer_langgraph(call, status, body),
            Platform::Langflow => self.answer_langflow(call, status, body),
        }
    }
}

fn a2a_request_value(call: &CompletionCall) -> Result<Value, HandlerError> {
    let url = a2a_request::get_complete_url(call.api_base.as_deref()).map_err(handler_failure)?;
    let headers = call.headers.clone().unwrap_or_default();
    let validated = a2a_request::validate_environment(&headers, call.api_key.as_ref());
    let body = a2a_request::transform_request(&call.messages, &call.optional_params, &RandomIds)
        .map_err(handler_failure)?;
    Ok(request_value(&url, &validated, &body))
}

fn langgraph_request_value(call: &CompletionCall) -> Result<Value, HandlerError> {
    let stream = call.stream == Some(true);
    let url = langgraph_config::get_complete_url(call.api_base.as_deref(), stream)
        .map_err(handler_failure)?;
    let mut headers = call.headers.clone().unwrap_or_default();
    langgraph_config::validate_environment(&mut headers, call.api_key.as_ref());
    // LiteLLM reads the body's stream_mode flag from litellm_params, while the endpoint follows the call's stream flag
    let body_stream = matches!(call.litellm_params.get("stream"), Some(Json::Bool(true)));
    let body = langgraph_request::transform_request(
        &call.model,
        &call.messages,
        &call.optional_params,
        body_stream,
    )
    .map_err(handler_failure)?;
    Ok(request_value(&url, &headers, &body))
}

fn langflow_request_value(call: &CompletionCall) -> Result<Value, HandlerError> {
    let prepared = langflow_call::prepare_run(&langflow_call::RunInput {
        model: &call.model,
        messages: &call.messages,
        optional_params: &call.optional_params,
        api_base: call.api_base.as_deref(),
        api_key: call.api_key.as_ref(),
        stream: call.stream,
    })
    .map_err(langflow_failure)?;
    Ok(request_value(
        &prepared.url,
        &prepared.headers,
        &prepared.body,
    ))
}

fn request_value(url: &str, headers: &BTreeMap<String, String>, body: &impl Serialize) -> Value {
    json!({"url": url, "headers": header_view(headers), "body": body})
}

fn header_view(headers: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    headers
        .iter()
        .filter(|(name, _)| KEPT_HEADERS.contains(&name.to_ascii_lowercase().as_str()))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.clone()))
        .collect()
}

fn handler_failure(failure: impl Display) -> HandlerError {
    HandlerError {
        status_code: None,
        message: failure.to_string(),
    }
}

fn langflow_failure(failure: LangFlowError) -> HandlerError {
    HandlerError {
        status_code: failure.status_code(),
        message: failure.to_string(),
    }
}

const fn role_text(role: LangGraphRole) -> &'static str {
    match role {
        LangGraphRole::Assistant => "assistant",
        LangGraphRole::Human => "human",
        LangGraphRole::System => "system",
    }
}
