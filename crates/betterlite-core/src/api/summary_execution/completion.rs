use std::time::Duration;

use async_trait::async_trait;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::chat::execution::ChatExecutor;
use betterlite_openai::chat::response::ChatResponse;
use betterlite_prompts::arguments::json::{Json, Object};
use serde::Serialize;
use serde_json::Number;

use crate::api::summary_execution::extract::{
    CompletionAnswer, CompletionChoice, CompletionMessage, CompletionUsage,
};

// the keyword arguments _call_summary_model hands to router.acompletion or litellm.acompletion
pub struct SummaryCall<'call> {
    pub model: &'call str,
    pub messages: &'call [Json],
    pub max_tokens: u64,
    pub timeout_seconds: f64,
    pub metadata: &'call Object,
    pub user: Option<&'call str>,
    pub allowed_model_region: Option<&'call str>,
}

// the exception text compact_20260112's "summary call failed" warning prints
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct SummaryCallFailure {
    pub message: String,
}

// B6020's Router.acompletion and B014's direct pipeline behind the one call the editor makes
#[async_trait]
pub trait SummaryCompletion: Send + Sync {
    async fn complete(&self, call: SummaryCall<'_>)
    -> Result<CompletionAnswer, SummaryCallFailure>;
}

// B014's chat completion pipeline as the direct caller: the deployment already resolved the summary model
pub struct ExecutorCompletion<'executor> {
    pub executor: &'executor ChatExecutor,
    pub deployment: &'executor ChatDeployment,
}

#[derive(Serialize)]
struct SummaryWire<'wire> {
    model: &'wire str,
    messages: &'wire [Json],
    max_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    user: Option<&'wire str>,
}

#[async_trait]
impl SummaryCompletion for ExecutorCompletion<'_> {
    async fn complete(
        &self,
        call: SummaryCall<'_>,
    ) -> Result<CompletionAnswer, SummaryCallFailure> {
        let wire = SummaryWire {
            model: call.model,
            messages: call.messages,
            max_tokens: call.max_tokens,
            user: call.user,
        };
        let body = serde_json::to_vec(&wire).map_err(|error| SummaryCallFailure {
            message: error.to_string(),
        })?;
        let request = ChatRequest::parse(&body).map_err(|error| SummaryCallFailure {
            message: error.to_string(),
        })?;
        let timeout = Duration::try_from_secs_f64(call.timeout_seconds).map_err(|_cannot| {
            SummaryCallFailure {
                message: format!(
                    "the summary timeout of {} seconds cannot be used",
                    call.timeout_seconds
                ),
            }
        })?;
        let mut deployment = self.deployment.clone();
        deployment.timeout = timeout;
        deployment.timeout_seconds = call.timeout_seconds;
        let outcome = self
            .executor
            .send(&deployment, &request)
            .await
            .map_err(|error| SummaryCallFailure {
                message: error.to_string(),
            })?;
        Ok(answer_from_response(outcome.response))
    }
}

fn answer_from_response(response: ChatResponse) -> CompletionAnswer {
    let content = response
        .choices
        .first()
        .and_then(|choice| choice.message.content.clone())
        .map(Json::Text);
    CompletionAnswer {
        choices: Some(vec![CompletionChoice {
            message: Some(CompletionMessage { content }),
        }]),
        usage: Some(CompletionUsage {
            prompt_tokens: Some(Json::Number(Number::from(response.usage.prompt_tokens))),
            completion_tokens: Some(Json::Number(Number::from(response.usage.completion_tokens))),
        }),
    }
}
