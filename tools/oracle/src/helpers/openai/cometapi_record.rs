use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_openai::chat::ChatMessage;
use betterlite_openai::chat_content::RefuseDownloads;
use betterlite_openai::cometapi_wire::chat::{
    CHAT_FAMILY, ChatError, ChatTransform, complete_url, transform_request,
};
use betterlite_openai::cometapi_wire::chunk::{ChunkError, CometApiChunk, chunk_parser};
use betterlite_openai::cometapi_wire::embed::{
    EmbeddingReplyError, EmbeddingReplyInput, complete_url as embedding_url,
    decode_comet_embedding_reply, map_openai_params as map_embedding_options,
    transform_embedding_request, validate_environment,
};
use betterlite_openai::openai_base_config::{
    CatalogCapabilities, Headers, OpenAiConfig, OpenAiSettings,
};
use betterlite_prompts::arguments::json::{Json, Object as JsonObject};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

mod tests;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Step {
    Options {
        model: String,
        #[serde(default)]
        params: Map<String, Value>,
        #[serde(default)]
        optional_params: Map<String, Value>,
        #[serde(default)]
        catalog: Catalog,
    },
    Request {
        model: String,
        #[serde(default)]
        messages: Vec<ChatMessage>,
        #[serde(default)]
        optional_params: JsonObject,
        #[serde(default)]
        env: BTreeMap<String, String>,
    },
    Url {
        #[serde(default)]
        api_base: Option<String>,
    },
    Chunk {
        chunk: JsonObject,
    },
    EmbeddingHeaders {
        #[serde(default)]
        api_key: Option<String>,
        #[serde(default)]
        headers: BTreeMap<String, String>,
        #[serde(default)]
        env: BTreeMap<String, String>,
    },
    EmbeddingUrl {
        #[serde(default)]
        api_base: Option<String>,
    },
    EmbeddingOptions {
        #[serde(default)]
        params: JsonObject,
        #[serde(default)]
        optional_params: JsonObject,
    },
    EmbeddingRequest {
        model: String,
        input: Json,
        #[serde(default)]
        optional_params: JsonObject,
    },
    EmbeddingReply {
        #[serde(default)]
        body: String,
        #[serde(default)]
        status: u16,
        #[serde(default)]
        headers: BTreeMap<String, String>,
    },
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    #[serde(default)]
    chat: Vec<String>,
    #[serde(default)]
    text: Vec<String>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        input
            .steps
            .iter()
            .map(step_value)
            .collect::<Result<Vec<Value>, Box<HelperError>>>()
            .map(Value::Array)
    }
}

fn invalid(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("InvalidInput"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn step_value(step: &Step) -> Result<Value, Box<HelperError>> {
    match step {
        Step::Options {
            model,
            params,
            optional_params,
            catalog,
        } => Ok(options(model, params, optional_params, catalog)),
        Step::Request {
            model,
            messages,
            optional_params,
            env,
        } => Ok(request(model, messages, optional_params, env)),
        Step::Url { api_base } => Ok(json!({"url": complete_url(api_base.as_deref())})),
        Step::Chunk { chunk } => Ok(chunk_step(chunk)),
        Step::EmbeddingHeaders { .. }
        | Step::EmbeddingUrl { .. }
        | Step::EmbeddingOptions { .. }
        | Step::EmbeddingRequest { .. }
        | Step::EmbeddingReply { .. } => embedding_step(step),
    }
}

fn embedding_step(step: &Step) -> Result<Value, Box<HelperError>> {
    match step {
        Step::EmbeddingHeaders {
            api_key,
            headers,
            env,
        } => embedding_headers(api_key.as_deref(), headers, env),
        Step::EmbeddingUrl { api_base } => Ok(json!({"url": embedding_url(api_base.as_deref())})),
        Step::EmbeddingOptions {
            params,
            optional_params,
        } => Ok(embedding_options(params, optional_params)),
        Step::EmbeddingRequest {
            model,
            input,
            optional_params,
        } => Ok(json!({
            "ok": true,
            "value": transform_embedding_request(model, input, optional_params),
        })),
        Step::EmbeddingReply {
            body,
            status,
            headers,
        } => Ok(embedding_reply(body, *status, headers)),
        Step::Options { .. } | Step::Request { .. } | Step::Url { .. } | Step::Chunk { .. } => Err(
            invalid(String::from("a chat step reached the embedding dispatch")),
        ),
    }
}

fn options(
    model: &str,
    params: &Map<String, Value>,
    optional_params: &Map<String, Value>,
    catalog: &Catalog,
) -> Value {
    let config = OpenAiConfig {
        family: CHAT_FAMILY,
        api_key: None,
        headers: Headers::new(),
        settings: OpenAiSettings::default(),
    };
    let capabilities = CatalogCapabilities {
        chat_models: catalog.chat.iter().cloned().collect(),
        text_models: catalog.text.iter().cloned().collect(),
    };
    let mut mapped = optional_params.clone();
    config.map_params(model, params, &mut mapped, &capabilities);
    json!({
        "supported": config.supported_params(model, &capabilities),
        "mapped": mapped,
    })
}

fn request(
    model: &str,
    messages: &[ChatMessage],
    optional_params: &JsonObject,
    env: &BTreeMap<String, String>,
) -> Value {
    let transform = ChatTransform {
        model,
        messages,
        optional_params,
        environment: env,
        downloads: &RefuseDownloads,
    };
    match transform_request(&transform) {
        Ok(body) => json!({"ok": true, "value": body}),
        Err(ChatError::ExtraBody | ChatError::Content(_)) => json!({"ok": false}),
    }
}

fn chunk_step(chunk: &JsonObject) -> Value {
    match chunk_parser(chunk) {
        Ok(parsed) => json!({"ok": true, "value": chunk_json(&parsed)}),
        Err(ChunkError::Api(error)) => json!({
            "ok": false,
            "class": "CometAPIException",
            "status": error.status_code,
        }),
        Err(ChunkError::FieldShape { field }) => {
            json!({"ok": false, "class": "InvalidChunk", "field": field})
        }
    }
}

fn chunk_json(chunk: &CometApiChunk) -> Value {
    json!({
        "id": chunk.id,
        "object": chunk.object,
        "created": chunk.created,
        "model": chunk.model,
        "usage": usage_json(chunk.usage.as_ref()),
        "choices": chunk.choices.iter().map(choice_json).collect::<Vec<Value>>(),
    })
}

fn usage_json(usage: Option<&Json>) -> Value {
    match usage {
        Some(Json::Object(fields)) => json!({
            "prompt_tokens": fields.get("prompt_tokens"),
            "completion_tokens": fields.get("completion_tokens"),
            "total_tokens": fields.get("total_tokens"),
        }),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Value::Null
        }
    }
}

fn choice_json(choice: &Json) -> Value {
    let Json::Object(fields) = choice else {
        return Value::Null;
    };
    let delta = fields.get("delta").and_then(Json::as_object);
    json!({
        "index": fields.get("index"),
        "content": delta.and_then(|delta_fields| delta_fields.get("content")),
        "reasoning": delta.and_then(|delta_fields| delta_fields.get("reasoning")),
        "reasoning_content": delta.and_then(|delta_fields| delta_fields.get("reasoning_content")),
    })
}

fn embedding_headers(
    api_key: Option<&str>,
    headers: &BTreeMap<String, String>,
    env: &BTreeMap<String, String>,
) -> Result<Value, Box<HelperError>> {
    let settings = ConfigLoader {
        lookup: env.clone(),
    }
    .load("")
    .map(|config| config.sections.cometapi_embedding_credentials)
    .map_err(|error| invalid(error.to_string()))?;
    let key = api_key.map(SecretString::from);
    Ok(json!({
        "headers": validate_environment(headers, key.as_ref(), &settings),
    }))
}

fn embedding_options(params: &JsonObject, optional_params: &JsonObject) -> Value {
    let mut mapped = optional_params.clone();
    map_embedding_options(params, &mut mapped);
    json!({"mapped": mapped})
}

fn embedding_reply(body: &str, status: u16, headers: &BTreeMap<String, String>) -> Value {
    let input = EmbeddingReplyInput {
        body: body.as_bytes(),
        status,
        headers,
    };
    match decode_comet_embedding_reply(&input) {
        Ok(reply) => json!({"ok": true, "value": reply}),
        Err(EmbeddingReplyError::Api(error)) => json!({
            "ok": false,
            "class": "CometAPIException",
            "status": error.status_code,
            "headers": carried_headers(headers, &error.headers),
        }),
        Err(EmbeddingReplyError::TooLarge { .. } | EmbeddingReplyError::FieldShape { .. }) => {
            json!({"ok": false, "class": "InvalidReply"})
        }
    }
}

fn carried_headers(
    supplied: &BTreeMap<String, String>,
    response_headers: &Headers,
) -> BTreeMap<String, Option<String>> {
    supplied
        .keys()
        .map(|name| (name.clone(), response_headers.get(name).cloned()))
        .collect()
}
