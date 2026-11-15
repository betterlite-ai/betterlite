use std::collections::BTreeMap;
use std::str;

use async_trait::async_trait;

use super::decode::decode_petals_reply;
use super::error::PetalsError;

// LiteLLM counts the rendered prompt and the output text with its default OpenAI encoding, so the caller supplies the counter (handler.py L130-L131)
#[async_trait]
pub trait TokenCounter: Send + Sync {
    async fn count_text(&self, text: &str) -> Option<i64>;
}

pub struct ReplyInput<'request> {
    pub body: &'request [u8],
    pub status: u16,
    pub headers: &'request BTreeMap<String, String>,
    pub prompt: Option<&'request str>,
    pub model: &'request str,
    pub created: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PetalsUsage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PetalsChoice {
    pub index: u64,
    pub finish_reason: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PetalsResponse {
    pub model: String,
    pub created: i64,
    pub choices: Vec<PetalsChoice>,
    pub usage: PetalsUsage,
}

pub async fn transform_response(
    reply: &ReplyInput<'_>,
    counter: &dyn TokenCounter,
) -> Result<PetalsResponse, PetalsError> {
    if !(200..300).contains(&reply.status) {
        return Err(status_error(reply));
    }
    let decoded = decode_petals_reply(reply.body).map_err(|source| PetalsError {
        status_code: 500,
        message: source.to_string(),
        headers: BTreeMap::new(),
    })?;
    // LiteLLM posts empty inputs for a model whose prompt factory returns None, then raises when it counts the None prompt (handler.py L130), so a working caller sees a 500
    let Some(prompt) = reply.prompt else {
        return Err(PetalsError {
            status_code: 500,
            message: String::from(
                "the petals prompt is missing, so the reply has no prompt to count",
            ),
            headers: BTreeMap::new(),
        });
    };
    let prompt_tokens = counter.count_text(prompt).await.unwrap_or(0);
    let completion_tokens = counter.count_text(&decoded.outputs).await.unwrap_or(0);
    Ok(PetalsResponse {
        model: reply.model.to_owned(),
        created: reply.created,
        choices: vec![PetalsChoice {
            index: 0,
            finish_reason: String::from("stop"),
            content: decoded.outputs,
        }],
        usage: PetalsUsage {
            prompt_tokens,
            completion_tokens,
            total_tokens: prompt_tokens.saturating_add(completion_tokens),
        },
    })
}

fn status_error(reply: &ReplyInput<'_>) -> PetalsError {
    PetalsError {
        status_code: error_status(reply.status),
        message: format!("PetalsException - {}", body_text(reply.body)),
        headers: reply.headers.clone(),
    }
}

// httpx raise_for_status rejects 1xx and 3xx too, and LiteLLM answers those with APIConnectionError status 500
const fn error_status(status: u16) -> u16 {
    if status >= 400 { status } else { 500 }
}

fn body_text(bytes: &[u8]) -> String {
    let Ok(text) = str::from_utf8(bytes) else {
        return format!("{} bytes that are not UTF-8", bytes.len());
    };
    text.to_owned()
}
