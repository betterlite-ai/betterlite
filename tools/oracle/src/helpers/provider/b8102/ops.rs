use async_trait::async_trait;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::{Content, Message, convert_content_list_to_str};
use betterlite_provider::langgraph_chat::config::{
    SHOULD_FAKE_STREAM, STREAM_PARAM_IN_BODY, SUPPORTED_OPENAI_PARAMS, get_complete_url,
    provider_info, validate_environment,
};
use betterlite_provider::langgraph_chat::reply::{
    LangGraphReply, ReplyInput, TokenCounter, transform_response,
};
use betterlite_provider::langgraph_chat::request::transform_request;
use betterlite_tokenizer::count::error::CountError;
use secrecy::ExposeSecret;
use serde_json::{Value, json};

use super::script::{
    HeadersCase, InfoCase, ReplyCase, RequestCase, UrlCase, helper_error, runtime,
};
use crate::helpers::HelperResult;

struct WordsCounter;

#[async_trait]
impl TokenCounter for WordsCounter {
    async fn count_messages(&self, _model: &str, messages: &[Message]) -> Result<u64, CountError> {
        Ok(messages
            .iter()
            .map(|message| {
                convert_content_list_to_str(message)
                    .split_whitespace()
                    .count()
            })
            .map(|count| u64::try_from(count).unwrap_or(u64::MAX))
            .fold(0_u64, u64::saturating_add))
    }

    async fn count_response_tokens(&self, _model: &str, text: &str) -> Result<u64, CountError> {
        Ok(u64::try_from(text.split_whitespace().count()).unwrap_or(u64::MAX))
    }
}

fn prompt() -> Vec<Message> {
    vec![Message {
        role: String::from("user"),
        content: Some(Content::Text(String::from("hello there"))),
        name: None,
        provider_specific_fields: Object::new(),
    }]
}

fn provider_json(case: &InfoCase) -> Value {
    let resolved = provider_info(
        case.api_base.as_deref(),
        case.api_key.as_ref(),
        case.env_api_base.as_deref(),
        case.env_api_key.as_ref(),
    );
    json!({
        "base": resolved.api_base,
        "key": resolved.api_key.as_ref().map(ExposeSecret::expose_secret),
    })
}

pub(super) fn info(cases: &[InfoCase]) -> Value {
    json!({
        "supported": SUPPORTED_OPENAI_PARAMS,
        "body_stream": STREAM_PARAM_IN_BODY,
        "fake_stream": SHOULD_FAKE_STREAM,
        "cases": cases.iter().map(provider_json).collect::<Vec<_>>(),
    })
}

pub(super) fn urls(cases: &[UrlCase]) -> Value {
    let mut found = Vec::with_capacity(cases.len());
    for case in cases {
        match get_complete_url(case.api_base.as_deref(), case.stream) {
            Ok(url) => found.push(json!({"ok": true, "url": url})),
            Err(_failure) => found.push(json!({"ok": false})),
        }
    }
    json!({"urls": found})
}

pub(super) fn headers(cases: &[HeadersCase]) -> Value {
    let mut found = Vec::with_capacity(cases.len());
    for case in cases {
        let mut merged = case.headers.clone();
        validate_environment(&mut merged, case.api_key.as_ref());
        found.push(merged);
    }
    json!({"headers": found})
}

pub(super) fn requests(cases: &[RequestCase]) -> HelperResult {
    let mut bodies = Vec::with_capacity(cases.len());
    for case in cases {
        let body = transform_request(&case.model, &case.messages, &case.options, case.stream)
            .map_err(|source| helper_error("LangGraphFailure", source.to_string()))?;
        bodies.push(
            serde_json::to_value(body)
                .map_err(|source| helper_error("Serialize", source.to_string()))?,
        );
    }
    Ok(json!({"bodies": bodies}))
}

fn reply_value(reply: &LangGraphReply) -> Value {
    reply.choices.first().map_or_else(
        || json!({"missing_choice": true}),
        |choice| {
            json!({
                "model": reply.model,
                "finish_reason": choice.finish_reason,
                "role": choice.role,
                "content": choice.content,
                "usage": reply.usage.map(|usage| [
                    usage.prompt_tokens,
                    usage.completion_tokens,
                    usage.total_tokens,
                ]),
            })
        },
    )
}

pub(super) fn replies(cases: &[ReplyCase]) -> HelperResult {
    let runtime = runtime()?;
    let messages = prompt();
    let mut found = Vec::with_capacity(cases.len());
    for case in cases {
        let input = ReplyInput {
            model: &case.model,
            status: case.status,
            body: case.body.as_bytes(),
            messages: &messages,
        };
        found.push(
            match runtime.block_on(transform_response(&input, &WordsCounter)) {
                Ok(reply) => json!({"ok": true, "reply": reply_value(&reply)}),
                Err(error) => json!({"ok": false, "status": error.status_code}),
            },
        );
    }
    Ok(json!({"replies": found}))
}
