use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_prompts::builtin::Message;
use betterlite_provider::petals::reply::{ReplyInput, TokenCounter, transform_response};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use crate::helpers::HelperResult;

use super::{FIXED_NOW, runtime};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyCases {
    replies: Vec<ReplyCase>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplyCase {
    model: String,
    body: String,
    #[serde(default = "default_status")]
    status: u16,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    messages: Vec<Message>,
}

const fn default_status() -> u16 {
    200
}

struct StandInCounter;

#[async_trait]
impl TokenCounter for StandInCounter {
    async fn count_messages(&self, messages: &[Message]) -> Option<i64> {
        Some(i64::try_from(messages.len()).unwrap_or(i64::MAX))
    }

    async fn count_text(&self, text: &str) -> Option<i64> {
        Some(i64::try_from(text.chars().count()).unwrap_or(i64::MAX))
    }
}

pub(crate) fn reply_values(input: &ReplyCases) -> HelperResult {
    let executor = runtime()?;
    let counter = StandInCounter;
    let replies: Vec<Value> = input
        .replies
        .iter()
        .map(|reply| one_reply(&executor, &counter, reply))
        .collect();
    Ok(json!({"replies": replies}))
}

fn one_reply(executor: &Runtime, counter: &StandInCounter, reply: &ReplyCase) -> Value {
    let input = ReplyInput {
        body: reply.body.as_bytes(),
        status: reply.status,
        headers: &reply.headers,
        messages: &reply.messages,
        model: &reply.model,
        created: FIXED_NOW,
    };
    match executor.block_on(transform_response(&input, counter)) {
        Ok(response) => json!({
            "ok": true,
            "model": response.model,
            "created": response.created,
            "choices": response.choices.iter().map(|choice| json!({
                "index": choice.index,
                "content": choice.content,
                "finish_reason": choice.finish_reason,
            })).collect::<Vec<Value>>(),
            "usage": {
                "prompt_tokens": response.usage.prompt_tokens,
                "completion_tokens": response.usage.completion_tokens,
                "total_tokens": response.usage.total_tokens,
            },
        }),
        Err(error) => json!({
            "ok": false,
            "status": error.status_code,
            "message": error.message,
        }),
    }
}
