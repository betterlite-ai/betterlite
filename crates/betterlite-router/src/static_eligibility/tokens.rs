use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::sections::local_ttl_cache::Settings;
use betterlite_context::local_ttl_cache::dual::{Diagnostics, DualLocalCache};
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::responses_input_messages::error::InputMessagesError;
use betterlite_core::api::responses_input_messages::{InputRequest, input_messages};
use betterlite_models::decisions::Json;
use betterlite_redact::error::RedactionError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::Message;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::static_eligibility::filter::Cause;
use crate::static_eligibility::params::declared_model;

// The deployment's declared context window, resolved over the catalog and its model_info override.
pub trait TokenLimits {
    fn max_input_tokens(&self, spec: &DeploymentSpec, model: &str) -> Option<usize>;
}

#[derive(Debug, thiserror::Error)]
pub enum TokenCountError {
    #[error("Either messages or input must be provided to count tokens")]
    NoInput,
    #[error("a message of the request does not read as a chat message")]
    Message(#[from] serde_json::Error),
    #[error("the Responses input does not convert to chat messages: {0}")]
    ResponsesInput(#[from] InputMessagesError),
    #[error("the Responses input count needs a cache redactor: {0}")]
    Redaction(#[from] RedactionError),
    #[error(transparent)]
    Counter(#[from] CountError),
}

#[async_trait]
pub trait TokenCounter {
    async fn count_messages(
        &self,
        messages: &[Message],
        tools: &[Json],
    ) -> Result<usize, TokenCountError>;
}

#[derive(Clone, Copy, Debug)]
pub struct TokenPayload<'payload> {
    pub messages: Option<&'payload [Json]>,
    pub input: Option<&'payload Json>,
    pub instructions: Option<&'payload str>,
    pub system: Option<&'payload Json>,
    pub tools: &'payload [Json],
}

#[derive(serde::Serialize)]
struct SystemMessage<'content> {
    role: &'static str,
    content: &'content Json,
}

// anthropic_system_to_openai_message: a non-empty system string or block list counts as one system message.
fn system_message(system: Option<&Json>) -> Result<Option<Message>, TokenCountError> {
    let selected = match system {
        Some(Json::Text(text)) if !text.is_empty() => system,
        Some(Json::List(items)) if !items.is_empty() => system,
        Some(
            Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)
            | Json::Object(_),
        )
        | None => None,
    };
    let Some(content) = selected else {
        return Ok(None);
    };
    let value = serde_json::to_value(SystemMessage {
        role: "system",
        content,
    })?;
    Ok(Some(serde_json::from_value(value)?))
}

// _count_pre_call_check_tokens: chat messages plus the Anthropic system block, or the Responses input through the shared transform.
pub async fn count_request_tokens<C>(
    payload: &TokenPayload<'_>,
    counter: &C,
) -> Result<usize, TokenCountError>
where
    C: TokenCounter + Sync + ?Sized,
{
    let mut wire_messages = Vec::new();
    if let Some(messages) = payload.messages {
        if let Some(message) = system_message(payload.system)? {
            wire_messages.push(message);
        }
        for message in messages {
            wire_messages.push(serde_json::from_value(serde_json::to_value(message)?)?);
        }
    } else {
        let request = InputRequest {
            input: Some(payload.input.cloned().ok_or(TokenCountError::NoInput)?),
            instructions: payload.instructions.map(str::to_owned),
        };
        let cache = DualLocalCache::new(
            None,
            None,
            &Settings::default(),
            Arc::new(|| 0.0_f64),
            Diagnostics {
                redactor: Arc::new(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)?),
                emit: Arc::new(|_event| {}),
            },
        );
        for message in input_messages(&request, false, ApiLimits::default(), &cache)? {
            wire_messages.push(serde_json::from_value(message)?);
        }
    }
    counter.count_messages(&wire_messages, payload.tools).await
}

// The off-loop pre-call count: no messages or no declared limit skips it, and a failed count leaves the deployments unfiltered.
pub async fn count_for_pre_call_checks<L, C>(
    model: &str,
    deployments: &[DeploymentSpec],
    payload: &TokenPayload<'_>,
    limits: &L,
    counter: &C,
) -> Option<usize>
where
    L: TokenLimits + Sync + ?Sized,
    C: TokenCounter + Sync + ?Sized,
{
    if payload.messages.is_none() && payload.input.is_none() {
        return None;
    }
    if !deployments
        .iter()
        .any(|deployment| limits.max_input_tokens(deployment, model).is_some())
    {
        return None;
    }
    match count_request_tokens(payload, counter).await {
        Ok(count) => Some(count),
        Err(error) => {
            tracing::error!("the pre-call token count failed: {error}");
            None
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error(
    "Context Window exceeded for given call. No models have context window large enough for this call.\n{causes}"
)]
pub struct ContextWindowExceeded {
    pub causes: String,
}

// What the context window kept, and the rejected-deployment text _pre_call_checks reports when every candidate is invalid.
#[must_use]
#[derive(Debug)]
pub struct ContextWindowFiltered {
    pub deployments: Vec<DeploymentSpec>,
    pub rejected: String,
}

// The context-window arm of _pre_call_checks: an over-window deployment leaves the pool and contributes its error text.
pub fn filter_context_window<L>(
    model: &str,
    deployments: Vec<DeploymentSpec>,
    input_tokens: Option<usize>,
    limits: &L,
    causes: &mut Vec<Cause>,
) -> ContextWindowFiltered
where
    L: TokenLimits + ?Sized,
{
    let Some(count) = input_tokens else {
        return ContextWindowFiltered {
            deployments,
            rejected: String::new(),
        };
    };
    let mut kept = Vec::new();
    let mut rejected: Vec<String> = Vec::new();
    for deployment in deployments {
        match limits.max_input_tokens(&deployment, model) {
            Some(maximum) if count > maximum => {
                causes.push(Cause::ContextWindow {
                    model_name: deployment.model_name.clone(),
                    max_input_tokens: maximum,
                    input_tokens: count,
                });
                rejected.push(format!(
                    "Model={}, Max Input Tokens={maximum}, Got={count}",
                    declared_model(&deployment).unwrap_or(model)
                ));
            }
            Some(_) | None => kept.push(deployment),
        }
    }
    ContextWindowFiltered {
        deployments: kept,
        rejected: rejected.concat(),
    }
}
