use betterlite_config::sections::chatgpt_headers::Settings as HeaderSettings;
use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::call_helpers::headers::{HeaderSource, process_response_headers};
use betterlite_models::decisions::Json as RecordJson;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::chatgpt::chat::Authenticator;
use betterlite_openai::chatgpt::headers::session::{
    LitellmParamsInput, SessionIdentity, ensure_chatgpt_session_id,
};
use betterlite_openai::chatgpt::headers::{
    PlatformEnvironment, get_chatgpt_default_headers, get_chatgpt_instructions,
};
use betterlite_openai::chatgpt::login::error::LoginError;
use betterlite_openai::error::OpenAiError;
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::response_events::created_at::CreatedAtError;
use betterlite_openai::response_events::{RawResponse, ResponsesReply, transform_response};
use betterlite_openai::response_input::error::ResponseInputError;
use betterlite_openai::response_input::tools::source_type_name;
use betterlite_openai::response_input::{ResponsesInput, transform_responses_request};
use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;
use thiserror::Error;

use crate::api::api_boundaries::ApiLimits;
use crate::api::responses_sse_recovery::ResponsesSseRecoveryError;

mod recovery;
mod tests;

// LiteLLM's custom_llm_provider for the ChatGPT Responses API
pub const PROVIDER: ProviderId = ProviderId::Chatgpt;
pub const SUPPORTS_NATIVE_WEBSOCKET: bool = false;

const CHATGPT_API_BASE: &str = "https://chatgpt.com/backend-api/codex";
const REASONING_ENCRYPTED_CONTENT: &str = "reasoning.encrypted_content";
const ALLOWED_REQUEST_KEYS: [&str; 11] = [
    "model",
    "input",
    "instructions",
    "stream",
    "store",
    "include",
    "tools",
    "tool_choice",
    "reasoning",
    "previous_response_id",
    "truncation",
];

// LiteLLM's ChatGPTResponsesAPIConfig: the access token comes from B1509's login and the headers from B1507
pub struct ResponsesConfig<A> {
    pub authenticator: A,
    pub header_settings: HeaderSettings,
}

pub struct EnvironmentRequest<'request> {
    pub headers: Headers,
    pub params: &'request LitellmParamsInput,
    pub model: &'request str,
    pub platform: &'request dyn PlatformEnvironment,
    pub session: &'request dyn SessionIdentity,
}

#[derive(Debug, Error)]
pub enum ChatGptResponsesError {
    // LiteLLM turns GetAccessTokenError into AuthenticationError and keeps its message
    #[error("{source}")]
    Authentication {
        model: String,
        #[source]
        source: LoginError,
    },
    // a device-code or refresh failure is not a GetAccessTokenError, so it keeps its own status
    #[error("the ChatGPT login failed for model {model}")]
    Login {
        model: String,
        #[source]
        source: LoginError,
    },
    #[error(transparent)]
    Request(#[from] ResponseInputError),
    #[error("'instructions' must be a string, but got {found} instead")]
    InstructionsNotText { found: &'static str },
    #[error("'include' must be a list, but got {found} instead")]
    IncludeNotList { found: &'static str },
    #[error(transparent)]
    Transform(#[from] OpenAiError),
    #[error("recovering the buffered SSE stream failed: {source}")]
    Recovery {
        #[source]
        source: ResponsesSseRecoveryError,
    },
    #[error("the ChatGPT reply is {observed} bytes; the maximum is {limit}")]
    ReplyTooLarge { observed: usize, limit: usize },
    #[error("the completed event's created_at cannot be converted into an integer timestamp")]
    CreatedAt(#[from] CreatedAtError),
    #[error("the completed event does not serialize: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}

impl ChatGptResponsesError {
    #[must_use]
    pub fn http_status(&self) -> Option<u16> {
        match self {
            Self::Authentication { .. } => Some(401),
            Self::Login { source, .. } => Some(source.status()),
            Self::Request(source) => source.http_status(),
            Self::InstructionsNotText { .. } | Self::IncludeNotList { .. } => Some(400),
            Self::Transform(OpenAiError::Provider(error)) => Some(error.status_code),
            Self::Transform(_) => None,
            Self::Recovery { source } => match source {
                ResponsesSseRecoveryError::Limit(error) => Some(error.fields().status),
                ResponsesSseRecoveryError::Utf8(_) => None,
            },
            Self::ReplyTooLarge { .. } => Some(502),
            Self::CreatedAt(_) | Self::Encode { .. } => None,
        }
    }
}

impl<A: Authenticator> ResponsesConfig<A> {
    pub async fn validate_environment(
        &mut self,
        request: EnvironmentRequest<'_>,
    ) -> Result<Headers, ChatGptResponsesError> {
        let EnvironmentRequest {
            headers,
            params,
            model,
            platform,
            session,
        } = request;
        let access_token =
            self.authenticator
                .access_token()
                .await
                .map_err(|source| match source {
                    LoginError::AccessToken(_) => ChatGptResponsesError::Authentication {
                        model: model.to_owned(),
                        source,
                    },
                    LoginError::DeviceCode(_) | LoginError::Refresh(_) => {
                        ChatGptResponsesError::Login {
                            model: model.to_owned(),
                            source,
                        }
                    }
                })?;
        let account_id = self.authenticator.account_id();
        let session_id = ensure_chatgpt_session_id(params, session);
        let mut merged = get_chatgpt_default_headers(
            &self.header_settings,
            platform,
            &access_token,
            account_id.as_deref(),
            Some(&session_id),
        );
        merged.extend(headers);
        Ok(merged)
    }

    // the request is forced into the Codex shape after B507's own preparation
    pub fn transform_request(
        &self,
        model: &str,
        input: &ResponsesInput,
        options: &Object,
        model_info: Option<&Object>,
    ) -> Result<Object, ChatGptResponsesError> {
        let mut request = transform_responses_request(model, input, options, PROVIDER, model_info)?;
        insert_instructions(
            &mut request,
            get_chatgpt_instructions(&self.header_settings),
        )?;
        request.insert(String::from("store"), Json::Bool(false));
        request.insert(String::from("stream"), Json::Bool(true));
        insert_encrypted_content(&mut request)?;
        request.retain(|key, _value| ALLOWED_REQUEST_KEYS.contains(&key.as_str()));
        Ok(request)
    }

    // an explicit base wins over the authenticator's, unlike the chat config
    #[must_use]
    pub fn get_complete_url(&self, api_base: Option<&str>) -> String {
        let chosen = api_base
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| self.authenticator.api_base());
        let base = if chosen.is_empty() {
            CHATGPT_API_BASE
        } else {
            chosen
        };
        format!("{}/responses", base.trim_end_matches('/'))
    }
}

// LiteLLM's response transform: buffered SSE is recovered here, and anything else is B508's
pub fn transform_reply(
    reply: &RawResponse<'_>,
    now: i64,
    state: &mut LoggingState,
    env: &PrePostEnv<'_>,
    limits: &ApiLimits,
) -> Result<ResponsesReply, ChatGptResponsesError> {
    if reply.text.len() > limits.max_json_bytes {
        return Err(ChatGptResponsesError::ReplyTooLarge {
            observed: reply.text.len(),
            limit: limits.max_json_bytes,
        });
    }
    if !recovery::should_parse_as_sse(reply.headers, reply.text) {
        return Ok(transform_response(reply, now, state, env)?);
    }
    post_call(
        state,
        &PostCall {
            original_response: RecordJson::Text(reply.text.to_owned()),
            input: RecordJson::Null,
            additional_args: AdditionalArgs {
                api_base: None,
                headers: None,
                complete_input_dict: Some(RawBody::Object(IndexMap::new())),
                request_str: None,
                openai_sdk: None,
                passthrough: IndexMap::new(),
            },
        },
        env,
    );
    let recovered = recovery::extract_completed(reply.text, now, limits)?;
    let Some(response) = recovered.response else {
        return Err(ChatGptResponsesError::Transform(
            recovery::missing_completion(reply, recovered.error_message),
        ));
    };
    Ok(ResponsesReply {
        response,
        headers: reply.headers.clone(),
        additional_headers: process_response_headers(HeaderSource::Upstream(reply.headers)),
    })
}

// the instruction check is substring containment, so a caller who already carries the base text keeps it once
fn insert_instructions(request: &mut Object, base: &str) -> Result<(), ChatGptResponsesError> {
    match request.get("instructions") {
        Some(Json::Text(existing)) => {
            if !existing.contains(base) {
                request.insert(
                    String::from("instructions"),
                    Json::Text(format!("{base}\n\n{existing}")),
                );
            }
        }
        None | Some(Json::Null) => {
            request.insert(String::from("instructions"), Json::Text(base.to_owned()));
        }
        Some(other) => {
            return Err(ChatGptResponsesError::InstructionsNotText {
                found: source_type_name(other),
            });
        }
    }
    Ok(())
}

fn insert_encrypted_content(request: &mut Object) -> Result<(), ChatGptResponsesError> {
    let mut include = match request.get("include") {
        None | Some(Json::Null) => Vec::new(),
        Some(Json::List(items)) => items.clone(),
        Some(other) => {
            return Err(ChatGptResponsesError::IncludeNotList {
                found: source_type_name(other),
            });
        }
    };
    if !include
        .iter()
        .any(|item| matches!(item, Json::Text(text) if text == REASONING_ENCRYPTED_CONTENT))
    {
        include.push(Json::Text(REASONING_ENCRYPTED_CONTENT.to_owned()));
    }
    request.insert(String::from("include"), Json::List(include));
    Ok(())
}
