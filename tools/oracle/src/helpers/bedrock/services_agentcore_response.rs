use std::future::{self, Future};
use std::io;

use betterlite_bedrock::agentcore_response::counter::TokenCounter;
use betterlite_bedrock::agentcore_response::transform::{
    AgentCoreResponseError, RawResponse, transform_agentcore_response,
};
use betterlite_openai::chat::response::ChatResponse;
use betterlite_tokenizer::count::request::{Message, TokenRequest};
use serde::Deserialize;
use serde_json::json;
use tokio::runtime::Builder;

mod tests;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
    model: String,
    messages: Vec<Message>,
    response: ChatResponse,
    tokens: Option<TokenCounts>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct TokenCounts {
    prompt: usize,
    completion: usize,
}

struct FixtureCounter {
    counts: Option<TokenCounts>,
}

impl TokenCounter for FixtureCounter {
    type Error = io::Error;

    fn count(&self, request: &TokenRequest<'_>) -> impl Future<Output = Result<usize, io::Error>> {
        let counted = match (
            request.messages.is_some(),
            request.text.is_some(),
            request.count_response_tokens,
        ) {
            (true, false, None | Some(false)) => self.counts.map(|counts| counts.prompt),
            (false, true, Some(true)) => self.counts.map(|counts| counts.completion),
            _ => {
                return future::ready(Err(io::Error::other(
                    "fixture counter called with an unexpected request",
                )));
            }
        };
        future::ready(counted.ok_or_else(|| io::Error::other("fixture counter unavailable")))
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| failure(error.to_string()))?
            .block_on(transform(input))
    }
}

async fn transform(input: &Input) -> crate::helpers::HelperResult {
    let raw = RawResponse {
        status_code: input.status,
        headers: input.headers.clone(),
        body: input.body.clone().into_bytes(),
    };
    let mut response = input.response.clone();
    let counter = FixtureCounter {
        counts: input.tokens,
    };
    match transform_agentcore_response(&input.model, &raw, &input.messages, &mut response, &counter)
        .await
    {
        Ok(()) => Ok(json!(Completed::from(&response))),
        Err(AgentCoreResponseError::Bedrock { status, error }) => Ok(json!({
            "error": {
                "status_code": status,
                "status_code_is_synthesized": error.cause.error.status_code_is_synthesized,
                "headers": error.cause.error.headers,
                "response_status": error.cause.error.response.status_code,
            },
        })),
        Err(AgentCoreResponseError::Header(error)) => Err(failure(error.to_string())),
    }
}

#[derive(serde::Serialize)]
struct Completed<'response> {
    id: &'response str,
    object: &'response str,
    created: u64,
    model: &'response str,
    system_fingerprint: Option<&'response str>,
    choices: Vec<CompletedChoice<'response>>,
    usage: CompletedUsage,
}

#[derive(serde::Serialize)]
struct CompletedChoice<'response> {
    index: u32,
    finish_reason: Option<&'response str>,
    role: &'response str,
    content: Option<&'response str>,
}

#[derive(serde::Serialize)]
struct CompletedUsage {
    #[serde(rename = "prompt_tokens")]
    prompt: u64,
    #[serde(rename = "completion_tokens")]
    completion: u64,
    #[serde(rename = "total_tokens")]
    total: u64,
}

impl<'response> From<&'response ChatResponse> for Completed<'response> {
    fn from(response: &'response ChatResponse) -> Self {
        Self {
            id: &response.id,
            object: &response.object,
            created: response.created,
            model: response.model.as_str(),
            system_fingerprint: response.system_fingerprint.as_deref(),
            choices: response
                .choices
                .iter()
                .map(|choice| CompletedChoice {
                    index: choice.index,
                    finish_reason: choice.finish_reason.as_deref(),
                    role: &choice.message.role,
                    content: choice.message.content.as_deref(),
                })
                .collect(),
            usage: CompletedUsage {
                prompt: response.usage.prompt_tokens,
                completion: response.usage.completion_tokens,
                total: response.usage.total_tokens,
            },
        }
    }
}

fn failure(message: String) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: "FixtureError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
