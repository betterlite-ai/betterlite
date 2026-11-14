use async_trait::async_trait;
use betterlite_models::decisions::Json;
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
    pub system: Option<&'payload Json>,
    pub tools: &'payload [Json],
}

// anthropic_system_to_openai_message: a non-empty system string or block list counts as one system message.
fn system_message(system: Option<&Json>) -> Result<Option<Message>, TokenCountError> {
    let content = match system {
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
    let Some(content) = content else {
        return Ok(None);
    };
    let mut object = serde_json::Map::new();
    object.insert(
        "role".to_owned(),
        serde_json::Value::String("system".to_owned()),
    );
    object.insert("content".to_owned(), serde_json::to_value(content)?);
    let message = serde_json::from_value(serde_json::Value::Object(object))?;
    Ok(Some(message))
}

// _count_pre_call_check_tokens: chat messages plus the Anthropic system block, counted with the request's tools.
pub async fn count_request_tokens<C>(
    payload: &TokenPayload<'_>,
    counter: &C,
) -> Result<usize, TokenCountError>
where
    C: TokenCounter + ?Sized,
{
    let Some(messages) = payload.messages else {
        return Err(TokenCountError::NoInput);
    };
    let mut counted = Vec::new();
    if let Some(message) = system_message(payload.system)? {
        counted.push(message);
    }
    for message in messages {
        let value = serde_json::to_value(message)?;
        counted.push(serde_json::from_value(value)?);
    }
    counter.count_messages(&counted, payload.tools).await
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
    L: TokenLimits + ?Sized,
    C: TokenCounter + ?Sized,
{
    if payload.messages.is_none() {
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

// The context-window arm of _pre_call_checks: an over-window deployment is out, and no deployment left is the error.
pub fn filter_context_window<L>(
    model: &str,
    deployments: Vec<DeploymentSpec>,
    input_tokens: Option<usize>,
    limits: &L,
    causes: &mut Vec<Cause>,
) -> Result<Vec<DeploymentSpec>, ContextWindowExceeded>
where
    L: TokenLimits + ?Sized,
{
    let Some(input_tokens) = input_tokens else {
        return Ok(deployments);
    };
    let mut kept = Vec::new();
    let mut rejected: Vec<String> = Vec::new();
    for deployment in deployments {
        match limits.max_input_tokens(&deployment, model) {
            Some(maximum) if input_tokens > maximum => {
                causes.push(Cause::ContextWindow {
                    model_name: deployment.model_name.clone(),
                    max_input_tokens: maximum,
                    input_tokens,
                });
                rejected.push(format!(
                    "Model={}, Max Input Tokens={maximum}, Got={input_tokens}",
                    declared_model(&deployment).unwrap_or(model)
                ));
            }
            Some(_) | None => kept.push(deployment),
        }
    }
    if !rejected.is_empty() && kept.is_empty() {
        return Err(ContextWindowExceeded {
            causes: rejected.concat(),
        });
    }
    Ok(kept)
}
