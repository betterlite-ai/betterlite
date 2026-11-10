use std::collections::BTreeMap;
use std::sync::Mutex;

use betterlite_openai::cohere_embedding::request::{
    CohereEmbeddingRequest, get_complete_url, map_openai_params, transform_embedding_request,
    transform_request, validate_environment,
};
use betterlite_openai::cohere_embedding::response::{
    TransformInput, transform_embedding_response,
};
use betterlite_openai::cohere_embedding::{
    DEFAULT_EMBEDDING_INPUT_TYPE, EmbeddingLogging, ModelCallDetails, PostCall,
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
struct Call {
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

struct Fixture {
    encoded: Mutex<Vec<String>>,
    fail_on: Option<String>,
    details: ModelCallDetails,
    calls: Vec<Value>,
}

impl TextEncoding for Fixture {
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

impl EmbeddingLogging for Fixture {
    fn model_call_details(&self) -> &ModelCallDetails {
        &self.details
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

fn secret(value: Option<&String>) -> Option<SecretString> {
    match value {
        Some(key) => Some(SecretString::from(key.clone())),
        None => None,
    }
}

fn fixture(fail_on: Option<String>, input: &[String]) -> Fixture {
    Fixture {
        encoded: Mutex::new(Vec::new()),
        fail_on,
        details: ModelCallDetails {
            input: input.to_vec(),
        },
        calls: Vec::new(),
    }
}

fn outcome(request: Option<CohereEmbeddingRequest>) -> Value {
    match request {
        Some(request) => json!({"ok": true, "value": request}),
        None => json!({"ok": false, "value": null}),
    }
}

fn wrapper(model: &str, raw: &Value, mapped: &Object) -> Value {
    outcome(
        serde_json::from_value::<EmbeddingInput>(raw.clone())
            .ok()
            .and_then(|typed| transform_embedding_request(model, &typed, mapped).ok()),
    )
}

fn lower(model: &str, raw: &Value, mapped: &Object) -> Value {
    let Value::Array(items) = raw else {
        return Value::Null;
    };
    let Some(texts) = items
        .iter()
        .map(|item| item.as_str().map(str::to_owned))
        .collect::<Option<Vec<String>>>()
    else {
        return Value::Null;
    };
    outcome(transform_request(model, &texts, mapped).ok())
}

fn request(call: &Call) -> Value {
    let mut mapped = map_openai_params(&call.options, Object::new());
    mapped.extend(call.inference.clone());
    let key = secret(call.api_key.as_ref());
    let calls: Vec<Value> = call
        .inputs
        .iter()
        .map(|raw| {
            json!({
                "wrapper": wrapper(&call.model, raw, &mapped),
                "lower": lower(&call.model, raw, &mapped),
            })
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
    let Ok(request) =
        transform_embedding_request(model, &EmbeddingInput::Texts(entry.input.clone()), &mapped)
    else {
        return json!({"ok": false, "value": null, "post_call": [], "encoded": []});
    };
    let key = secret(entry.api_key.as_ref());
    let encoding = fixture(entry.fail_on.clone(), &entry.input);
    let mut logging = fixture(None, &entry.input);
    let result = transform_embedding_response(
        TransformInput {
            body: entry.body.as_bytes(),
            model,
            api_key: key.as_ref(),
            request_data: &request,
            encoding: &encoding,
        },
        &mut logging,
    );
    let encoded = encoding
        .encoded
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_default();
    let mut value = json!({
        "ok": result.is_ok(),
        "value": null,
        "post_call": logging.calls,
        "encoded": encoded,
    });
    if let Ok(response) = result {
        value["value"] = json!(response);
    }
    value
}

impl crate::helpers::Helper for Op {
    type Input = Call;

    fn run(call: &Self::Input) -> crate::helpers::HelperResult {
        Ok(match call.function {
            Function::Request => request(call),
            Function::Response => json!({
                "calls": call.requests.iter().map(|entry| reply(&call.model, entry)).collect::<Vec<Value>>(),
            }),
        })
    }
}

#[path = "cohere_embedding/tests.rs"]
mod tests;
