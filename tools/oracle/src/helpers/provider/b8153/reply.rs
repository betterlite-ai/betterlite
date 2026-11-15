use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_prompts::arguments::json::Object as Params;
use betterlite_prompts::builtin::Message;
use betterlite_provider::petals::reply::{ReplyInput, TokenCounter, transform_response};
use betterlite_provider::petals::request::RequestContext;
use betterlite_tokenizer::engine::assets;
use betterlite_tokenizer::engine::tiktoken::{Encoding, TiktokenEngine};
use chrono::DateTime;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use crate::helpers::HelperResult;

use super::{FIXED_NOW, petals_config, prompts, runtime};

const PETALS_BASE: &str = "http://petals.test/v1/generate";

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

// LiteLLM counts the rendered prompt and the output with the default OpenAI encoding, cl100k_base (main.py's _get_encoding)
struct DefaultEncodingCounter;

#[async_trait]
impl TokenCounter for DefaultEncodingCounter {
    async fn count_text(&self, text: &str) -> Option<i64> {
        let data = assets::bundled(Encoding::Cl100kBase)?;
        let engine = TiktokenEngine::from_bpe_file(Encoding::Cl100kBase, &data).ok()?;
        let tokens = engine.encode(text).ok()?;
        i64::try_from(tokens.len()).ok()
    }
}

pub(crate) fn reply_values(input: &ReplyCases) -> HelperResult {
    let executor = runtime()?;
    let config = petals_config(&BTreeMap::new())?;
    let prompts = prompts();
    let custom = BTreeMap::new();
    let context = RequestContext {
        config: &config,
        prompts: &prompts,
        custom_prompts: &custom,
        now: DateTime::from_timestamp(FIXED_NOW, 0).unwrap_or_default(),
    };
    let replies: Vec<Value> = input
        .replies
        .iter()
        .map(|reply| one_reply(&executor, &context, reply))
        .collect();
    Ok(json!({"replies": replies}))
}

fn one_reply(executor: &Runtime, context: &RequestContext<'_>, reply: &ReplyCase) -> Value {
    let prompt = match executor.block_on(context.transform_request(
        &reply.model,
        &reply.messages,
        Some(PETALS_BASE),
        &Params::new(),
    )) {
        Ok(body) => body.inputs,
        Err(_) => return json!({"ok": false, "status": 500}),
    };
    let input = ReplyInput {
        body: reply.body.as_bytes(),
        status: reply.status,
        headers: &reply.headers,
        prompt: prompt.as_deref(),
        model: &reply.model,
        created: FIXED_NOW,
    };
    match executor.block_on(transform_response(&input, &DefaultEncodingCounter)) {
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
        Err(error) => json!({"ok": false, "status": error.status_code}),
    }
}
