use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_openai::embedding::EmbeddingResponse;
use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::factory::custom::CustomPromptTemplate;
use betterlite_prompts::message_content::Message;
use betterlite_provider::ollama_chat::ids::RandomIds;
use betterlite_provider::ollama_chat::reply::{OllamaChatResponse, ToolCall};
use betterlite_provider::ollama_chat::usage::{TokenCounter, Usage};
use betterlite_provider::ollama_generate::config::{
    OllamaGenerateConfig, REQUIRED_PARAMS, SUPPORTED_OPENAI_PARAMS, complete_url, map_openai_params,
};
use betterlite_provider::ollama_generate::embed::{
    OllamaEmbeddingRequest, TokenEncoder, embed_url, transform_embedding_request,
    transform_embedding_response,
};
use betterlite_provider::ollama_generate::error::OllamaGenerateError;
use betterlite_provider::ollama_generate::reply::{ReplyInput, transform_response};
use betterlite_provider::ollama_generate::request::{
    OllamaGenerateRequest, RequestInput, RequestParams, transform_request,
};
use betterlite_provider::ollama_generate::stream::{
    GenerateStreamChunk, GenerateStreamStep, OllamaGenerateIterator,
};
use betterlite_tokenizer::count::error::CountError;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::helpers::Helper;
use crate::helpers::HelperError;
use crate::helpers::HelperResult;

const NOW: i64 = 1_700_000_000;

#[derive(Deserialize)]
pub(crate) struct ParamsInput {
    #[serde(default)]
    config: OllamaGenerateConfig,
    #[serde(default)]
    cases: Vec<ParamCase>,
    #[serde(default)]
    urls: Vec<Option<String>>,
}

#[derive(Deserialize)]
pub(crate) struct ParamCase {
    model: String,
    non_default: IndexMap<String, Json>,
    #[serde(default)]
    optional: IndexMap<String, Json>,
}

#[derive(Deserialize)]
pub(crate) struct RequestCase {
    model: String,
    messages: Vec<Message>,
    #[serde(default)]
    options: IndexMap<String, Json>,
    #[serde(default)]
    litellm_params: RequestParams,
}

#[derive(Deserialize)]
pub(crate) struct ReplyCase {
    model: String,
    body: String,
    #[serde(default)]
    request_data: IndexMap<String, Json>,
}

#[derive(Deserialize, Default, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EncoderKind {
    #[default]
    None,
    Words,
    Failing,
}

#[derive(Deserialize)]
pub(crate) struct EmbeddingCase {
    api_base: String,
    model: String,
    prompts: Vec<String>,
    #[serde(default)]
    options: IndexMap<String, Json>,
    status: u16,
    body: String,
    #[serde(default)]
    encoder: EncoderKind,
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(crate) enum Input {
    Params(Box<ParamsInput>),
    Request {
        #[serde(default)]
        custom_prompt_dict: BTreeMap<String, CustomPromptTemplate>,
        #[serde(default)]
        requests: Vec<RequestCase>,
    },
    Reply {
        #[serde(default)]
        replies: Vec<ReplyCase>,
    },
    Stream {
        #[serde(default)]
        lines: Vec<String>,
    },
    Embedding {
        #[serde(default)]
        cases: Vec<EmbeddingCase>,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Params(params) => Ok(param_cases(&params.config, &params.cases, &params.urls)),
            Input::Request {
                custom_prompt_dict,
                requests,
            } => Ok(request_cases(custom_prompt_dict, requests)),
            Input::Reply { replies } => reply_cases(replies),
            Input::Stream { lines } => Ok(stream_cases(lines)),
            Input::Embedding { cases } => embedding_cases(cases),
        }
    }
}

struct WordsCounter;

#[async_trait]
impl TokenCounter for WordsCounter {
    async fn count_messages(&self, _messages: &[Message]) -> Option<u64> {
        None
    }

    async fn count_text(&self, text: &str) -> Option<u64> {
        let words = text.split_whitespace().count();
        let count = u64::try_from(words).unwrap_or(u64::MAX);
        Some(count)
    }
}

struct WordsEncoder;

#[async_trait]
impl TokenEncoder for WordsEncoder {
    async fn encode(&self, text: &str) -> Result<Vec<u32>, CountError> {
        let count = text.split_whitespace().count();
        Ok(vec![1_u32; count])
    }
}

struct FailingEncoder;

#[async_trait]
impl TokenEncoder for FailingEncoder {
    async fn encode(&self, _text: &str) -> Result<Vec<u32>, CountError> {
        Err(CountError::Overflow)
    }
}

fn runtime() -> Result<tokio::runtime::Runtime, Box<HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|source| {
            Box::new(HelperError {
                class: String::from("Runtime"),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: source.to_string(),
            })
        })
}

fn failure(error: &OllamaGenerateError) -> Value {
    json!({
        "ok": false,
        "status": error.status()
    })
}

fn param_cases(
    config: &OllamaGenerateConfig,
    cases: &[ParamCase],
    urls: &[Option<String>],
) -> Value {
    let mapped: Vec<Value> = cases
        .iter()
        .map(|case| {
            let mut optional = case.optional.clone();
            match map_openai_params(&case.non_default, &mut optional, &case.model) {
                Ok(()) => json!({"ok": true, "options": optional}),
                Err(error) => failure(&error),
            }
        })
        .collect();
    let found_urls: Vec<String> = urls
        .iter()
        .map(|api_base| complete_url(api_base.as_deref()))
        .collect();
    json!({
        "supported": SUPPORTED_OPENAI_PARAMS,
        "required": REQUIRED_PARAMS,
        "config": config.get_config(),
        "mapped": mapped,
        "urls": found_urls,
    })
}

fn request_cases(
    custom_prompts: &BTreeMap<String, CustomPromptTemplate>,
    cases: &[RequestCase],
) -> Value {
    let bodies: Vec<Value> = cases
        .iter()
        .map(|case| {
            let input = RequestInput {
                model: &case.model,
                messages: &case.messages,
                optional_params: &case.options,
                params: &case.litellm_params,
                custom_prompts,
            };
            match transform_request(&input) {
                Ok(body) => json!({"ok": true, "body": body}),
                Err(error) => failure(&error),
            }
        })
        .collect();
    json!({"bodies": bodies})
}

fn reply_cases(cases: &[ReplyCase]) -> HelperResult {
    let runtime = runtime()?;
    let counter = WordsCounter;
    let ids = RandomIds;
    let mut found = Vec::with_capacity(cases.len());
    for case in cases {
        let input = ReplyInput {
            model: &case.model,
            body: case.body.as_bytes(),
            request_data: &case.request_data,
            created: NOW,
        };
        found.push(
            match runtime.block_on(transform_response(&input, &counter, &ids)) {
                Ok(response) => json!({"ok": true, "reply": reply_value(&response)}),
                Err(error) => failure(&error),
            },
        );
    }
    Ok(json!({"replies": found}))
}

fn reply_value(response: &OllamaChatResponse) -> Value {
    let Some(choice) = response.choices.first() else {
        return json!({"missing_choice": true});
    };
    let message = &choice.message;
    json!({
        "model": response.model,
        "created": response.created,
        "finish_reason": choice.finish_reason,
        "role": message.role,
        "content": content_value(message.content.as_deref()),
        "reasoning_content": message.reasoning_content,
        "tool_calls": message.tool_calls.as_deref().map(calls_value),
        "usage": [response.usage.prompt_tokens, response.usage.completion_tokens, response.usage.total_tokens],
    })
}

fn content_value(content: Option<&str>) -> Value {
    match content {
        None => Value::Null,
        Some(text) => serde_json::from_str::<Value>(text)
            .unwrap_or_else(|_error| Value::String(text.to_owned())),
    }
}

fn calls_value(calls: &[ToolCall]) -> Value {
    let values: Vec<Value> = calls
        .iter()
        .map(|call| {
            json!({
                "id": normalize_id(&call.id),
                "type": call.kind,
                "name": call.function.name,
                "arguments": normalize_arguments(&call.function.arguments),
            })
        })
        .collect();
    Value::Array(values)
}

fn normalize_arguments(arguments: &str) -> Value {
    serde_json::from_str::<Value>(arguments).map_or_else(
        |_error| Value::String(arguments.to_owned()),
        |value| Value::String(serde_json::to_string(&value).unwrap_or_default()),
    )
}

fn normalize_id(id: &str) -> String {
    for prefix in ["call_", "chatcmpl-"] {
        if let Some(rest) = id.strip_prefix(prefix) {
            if Uuid::parse_str(rest).is_ok() {
                return format!("{prefix}<uuid>");
            }
        }
    }
    if Uuid::parse_str(id).is_ok() {
        return String::from("<uuid>");
    }
    id.to_owned()
}

fn stream_cases(lines: &[String]) -> Value {
    let mut iterator = OllamaGenerateIterator::default();
    let ids = RandomIds;
    let chunks: Vec<Value> = lines
        .iter()
        .map(
            |line| match iterator.chunk_parser(line.as_bytes(), NOW, &ids) {
                Ok(GenerateStreamStep::Done(done)) => json!({
                    "ok": true,
                    "done": true,
                    "text": done.text,
                    "is_finished": done.is_finished,
                    "finish_reason": done.finish_reason,
                    "usage": done.usage.map(usage_value),
                }),
                Ok(GenerateStreamStep::Chunk(chunk)) => {
                    json!({"ok": true, "chunk": chunk_value(&chunk)})
                }
                Err(error) => json!({"ok": false, "status": error.status()}),
            },
        )
        .collect();
    json!({"chunks": chunks})
}

fn chunk_value(chunk: &GenerateStreamChunk) -> Value {
    let choices: Vec<Value> = chunk
        .choices
        .iter()
        .map(|choice| {
            json!({
                "index": choice.index,
                "finish_reason": choice.finish_reason,
                "content": choice.delta.content,
                "reasoning_content": choice.delta.reasoning_content,
            })
        })
        .collect();
    json!({
        "id": normalize_id(&chunk.id),
        "object": chunk.object,
        "created": chunk.created,
        "model": chunk.model,
        "usage": chunk.usage.map(usage_value),
        "choices": choices,
    })
}

fn usage_value(usage: Usage) -> Value {
    json!([
        usage.prompt_tokens,
        usage.completion_tokens,
        usage.total_tokens
    ])
}

fn embedding_cases(cases: &[EmbeddingCase]) -> HelperResult {
    let runtime = runtime()?;
    let mut found = Vec::with_capacity(cases.len());
    for case in cases {
        found.push(runtime.block_on(embedding_case(case)));
    }
    Ok(json!({"cases": found}))
}

async fn embedding_case(case: &EmbeddingCase) -> Value {
    let url = embed_url(&case.api_base);
    let payload: OllamaEmbeddingRequest =
        match transform_embedding_request(&case.model, &case.prompts, &case.options) {
            Ok(payload) => payload,
            Err(error) => return failure(&error),
        };
    let words = WordsEncoder;
    let failing = FailingEncoder;
    let encoder: Option<&dyn TokenEncoder> = match case.encoder {
        EncoderKind::None => None,
        EncoderKind::Words => Some(&words),
        EncoderKind::Failing => Some(&failing),
    };
    match transform_embedding_response(
        case.status,
        case.body.as_bytes(),
        &case.model,
        &case.prompts,
        encoder,
    )
    .await
    {
        Ok(response) => json!({
            "ok": true,
            "url": url,
            "payload": payload,
            "reply": embedding_value(&response),
        }),
        Err(error) => failure(&error),
    }
}

fn embedding_value(response: &EmbeddingResponse) -> Value {
    let data: Vec<Value> = response
        .data
        .iter()
        .map(|record| {
            json!({
                "object": record.object,
                "index": record.index,
                "embedding": serde_json::to_value(&record.embedding).unwrap_or(Value::Null),
            })
        })
        .collect();
    json!({
        "object": response.object,
        "model": response.model,
        "data": data,
        "usage": [
            response.usage.prompt_tokens,
            response.usage.completion_tokens,
            response.usage.total_tokens,
        ],
        "prompt_tokens_details": response.usage.prompt_tokens_details,
        "completion_tokens_details": response.usage.completion_tokens_details,
    })
}
