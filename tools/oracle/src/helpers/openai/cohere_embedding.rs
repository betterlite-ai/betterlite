use std::collections::BTreeMap;
use std::sync::Mutex;

use betterlite_openai::cohere_embedding::request::{
    get_complete_url, map_openai_params, transform_embedding_request, transform_request,
    validate_environment,
};
use betterlite_openai::cohere_embedding::response::{DecodeInput, transform_embedding_response};
use betterlite_openai::cohere_embedding::{
    CohereEmbeddingError, DEFAULT_EMBEDDING_INPUT_TYPE, EmbeddingLogging, PostCall,
    SUPPORTED_OPENAI_PARAMS, TextEncoding,
};
use betterlite_openai::embedding::EmbeddingInput;
use betterlite_prompts::arguments::json::Object;
use betterlite_tokenizer::engine::error::EngineError;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    Request,
    Response,
}

#[derive(Deserialize)]
pub(crate) struct Call {
    function: Function,
    model: String,
    #[serde(default)]
    options: Object,
    #[serde(default)]
    inference: Object,
    api_base: Option<String>,
    api_key: Option<String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    inputs: Vec<Value>,
    #[serde(default)]
    requests: Vec<Reply>,
}

#[derive(Deserialize)]
struct Reply {
    body: String,
    input: Vec<String>,
    #[serde(default)]
    options: Object,
    api_key: Option<String>,
    fail_on: Option<String>,
}

struct Encoder {
    encoded: Mutex<Vec<String>>,
    fail_on: Option<String>,
}

struct Recorder {
    input: Vec<String>,
    calls: Vec<Value>,
}

impl TextEncoding for Encoder {
    fn count_tokens(&self, text: &str) -> Result<usize, EngineError> {
        let Ok(mut encoded) = self.encoded.lock() else {
            return Err(EngineError::PoolClosed);
        };
        encoded.push(text.to_owned());
        if self.fail_on.as_deref() == Some(text) {
            return Err(EngineError::InputTooLarge {
                bytes: text.len(),
                limit: 0,
            });
        }
        Ok(text.chars().count())
    }
}

impl EmbeddingLogging for Recorder {
    fn input(&self) -> &[String] {
        &self.input
    }

    fn post_call(&mut self, call: PostCall<'_>) {
        self.calls.push(json!({
            "input": call.input,
            "api_key": call.api_key.map(|key| key.expose_secret().to_owned()),
            "complete_input_dict": call.complete_input_dict,
            "original_response": call.original_response,
        }));
    }
}

fn request(call: &Call) -> Value {
    let mut mapped = map_openai_params(&call.options, Object::new());
    mapped.extend(call.inference.clone());
    let key = call.api_key.clone().map(SecretString::from);
    let calls: Vec<Value> = call
        .inputs
        .iter()
        .map(|raw| {
            let built = serde_json::from_value::<EmbeddingInput>(raw.clone())
                .map_err(CohereEmbeddingError::from)
                .and_then(|typed| transform_embedding_request(&call.model, &typed, &mapped));
            let lower = serde_json::from_value::<Vec<String>>(raw.clone())
                .ok()
                .map(|strings| json!({"ok": true, "value": transform_request(&call.model, &strings, &mapped)}));
            let wrapper = built.map_or_else(
                |_| json!({"ok": false, "value": null}),
                |value| json!({"ok": true, "value": value}),
            );
            json!({"wrapper": wrapper, "lower": lower})
        })
        .collect();
    json!({
        "default_input_type": DEFAULT_EMBEDDING_INPUT_TYPE,
        "supported": SUPPORTED_OPENAI_PARAMS,
        "options": mapped,
        "headers": validate_environment(call.headers.clone(), key.as_ref()),
        "url": get_complete_url(call.api_base.as_deref()),
        "calls": calls,
    })
}

fn reply(model: &str, entry: &Reply) -> Value {
    let mapped = map_openai_params(&entry.options, Object::new());
    let input = EmbeddingInput::Texts(entry.input.clone());
    let key = entry.api_key.clone().map(SecretString::from);
    let encoding = Encoder {
        encoded: Mutex::new(Vec::new()),
        fail_on: entry.fail_on.clone(),
    };
    let mut logging = Recorder {
        input: entry.input.clone(),
        calls: Vec::new(),
    };
    let result = transform_embedding_request(model, &input, &mapped).and_then(|request| {
        let reply = DecodeInput {
            body: entry.body.as_bytes(),
            model,
            api_key: key.as_ref(),
            request_data: &request,
            encoding: &encoding,
        };
        transform_embedding_response(reply, &mut logging)
    });
    let encoded = encoding.encoded.lock().map(|guard| guard.clone());
    let value = result.map(|response| {
        let usage = &response.usage;
        json!({
            "object": response.object,
            "data": response.data,
            "model": response.model,
            "usage": [usage.prompt_tokens, usage.completion_tokens, usage.total_tokens],
            "billed": usage.prompt_tokens_details.map(|billed| [billed.text_tokens, billed.image_tokens]),
        })
    });
    json!({
        "ok": value.is_ok(),
        "value": value.ok(),
        "post_call": logging.calls,
        "encoded": encoded.unwrap_or_default(),
    })
}

impl crate::helpers::Helper for Op {
    type Input = Call;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        Ok(match input.function {
            Function::Request => request(input),
            Function::Response => {
                let calls = input
                    .requests
                    .iter()
                    .map(|entry| reply(&input.model, entry));
                json!({"calls": calls.collect::<Vec<_>>()})
            }
        })
    }
}

#[path = "cohere_embedding/tests.rs"]
mod tests;
