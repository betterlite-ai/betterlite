use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::cohere_embedding::TextEncoding;
use betterlite_openai::cohere_legacy_embedding::decode::LegacyEmbeddingResponse;
use betterlite_openai::cohere_legacy_embedding::error::CohereLegacyEmbeddingError;
use betterlite_openai::cohere_legacy_embedding::execute::{
    ExecuteInput, LegacyEmbeddingTransport, execute,
};
use betterlite_openai::cohere_legacy_embedding::plan::{PlanInput, plan};
use betterlite_openai::cohere_legacy_embedding::{EmbeddingHooks, PostCall, PreCall, SdkMode};
use betterlite_prompts::arguments::json::Object;
use betterlite_tokenizer::engine::error::EngineError;
use http::HeaderMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Runtime;

pub(crate) struct Op;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Call {
    model: String,
    #[serde(default)]
    input: Vec<String>,
    #[serde(default)]
    optional_params: Object,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    api_key: Option<String>,
    runs: Vec<Run>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Run {
    #[serde(default)]
    data: Option<Object>,
    status: u16,
    body: String,
}
struct Fixture {
    status: u16,
    body: Vec<u8>,
}
impl LegacyEmbeddingTransport for Fixture {
    fn send(
        &self,
        _request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        ready(Ok(UpstreamResponse {
            status: self.status,
            headers: HeaderMap::new(),
            body: self.body.clone(),
        }))
    }
}
impl TextEncoding for Fixture {
    fn count_tokens(&self, text: &str) -> Result<usize, EngineError> {
        Ok(text.chars().count())
    }
}

#[derive(Default)]
struct Recorder {
    records: Vec<Value>,
}

fn record(phase: &str, input: &[String], api_key: Option<&SecretString>, args: &Value) -> Value {
    json!({
        "phase": phase,
        "input": input,
        "api_key": api_key.map(|key| key.expose_secret().to_owned()),
        "additional_args": args,
    })
}

impl EmbeddingHooks for Recorder {
    fn pre_call(&mut self, call: PreCall<'_>) {
        let args = json!({
            "complete_input_dict": call.body,
            "headers": call.headers,
            "api_base": call.api_base,
        });
        self.records
            .push(record("pre_call", call.input, call.api_key, &args));
    }

    fn post_call(&mut self, call: PostCall<'_>) {
        let args = json!({"complete_input_dict": call.body});
        let mut value = record("post_call", call.input, call.api_key, &args);
        if let Value::Object(fields) = &mut value {
            fields.insert(
                String::from("original_response"),
                json!(call.original_response),
            );
        }
        self.records.push(value);
    }
}

fn outcome(value: Result<LegacyEmbeddingResponse, CohereLegacyEmbeddingError>) -> Value {
    match value {
        Ok(response) => {
            let usage = &response.usage;
            json!({
                "ok": true,
                "value": {
                    "object": response.object,
                    "data": response.data,
                    "model": response.model,
                    "usage": [usage.prompt_tokens, usage.completion_tokens, usage.total_tokens],
                    "billed": usage.prompt_tokens_details.map(|billed| [billed.text_tokens, billed.image_tokens]),
                },
            })
        }
        Err(CohereLegacyEmbeddingError::Status(error)) => {
            json!({"ok": false, "status": error.status_code})
        }
        Err(_other) => json!({"ok": false, "status": null}),
    }
}

fn scenario(runtime: &Runtime, input: &Call, run: &Run) -> Value {
    let key = input.api_key.clone().map(SecretString::from);
    let plan = plan(&PlanInput {
        mode: SdkMode::Async,
        model: &input.model,
        input: &input.input,
        optional_params: &input.optional_params,
        supplied_data: run.data.as_ref(),
        complete_api_base: None,
        api_key: key.as_ref(),
        headers: &input.headers,
        timeout: Duration::from_secs(600),
    });
    let fixture = Fixture {
        status: run.status,
        body: run.body.as_bytes().to_vec(),
    };
    let mut recorder = Recorder::default();
    let value = runtime.block_on(execute(
        &fixture,
        ExecuteInput {
            plan: &plan,
            input: &input.input,
            api_key: key.as_ref(),
            encoding: &fixture,
        },
        &mut recorder,
    ));
    json!({"hooks": recorder.records, "outcome": outcome(value)})
}

impl crate::helpers::Helper for Op {
    type Input = Call;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(|error| {
                Box::new(crate::helpers::HelperError {
                    class: String::from("CohereLegacyEmbedError"),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                    message: error.to_string(),
                })
            })?;
        let runs: Vec<Value> = input
            .runs
            .iter()
            .map(|run| scenario(&runtime, input, run))
            .collect();
        Ok(json!({"runs": runs}))
    }
}

#[path = "cohere_legacy_embed/tests.rs"]
mod tests;
