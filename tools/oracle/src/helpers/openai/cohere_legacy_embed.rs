use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::Mutex;
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
use betterlite_openai::cohere_legacy_embedding::plan::{LegacyEmbeddingPlan, PlanInput, plan};
use betterlite_openai::cohere_legacy_embedding::{EmbeddingHooks, PostCall, PreCall, SdkMode};
use betterlite_prompts::arguments::json::Object;
use betterlite_tokenizer::engine::error::EngineError;
use http::HeaderMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tokio::runtime::Runtime;

// a supplied client keeps its own timeout, so only the sync plan's client default could be read
const HELPER_TIMEOUT: Duration = Duration::from_secs(600);

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Call {
    mode: Mode,
    model: String,
    #[serde(default)]
    input: Vec<String>,
    #[serde(default)]
    optional_params: Object,
    runs: Vec<Run>,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum Mode {
    Sync,
    Async,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Run {
    #[serde(default)]
    data: Option<Object>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    complete_api_base: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    reply: Reply,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    status: u16,
    body: String,
}

struct Fixture {
    status: u16,
    body: Vec<u8>,
    seen: Mutex<Vec<OutboundRequest>>,
}

impl LegacyEmbeddingTransport for Fixture {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        let Ok(mut seen) = self.seen.lock() else {
            return ready(Err(TransportError::Closed));
        };
        seen.push(request);
        ready(Ok(UpstreamResponse {
            status: self.status,
            headers: HeaderMap::new(),
            body: self.body.clone(),
        }))
    }
}

#[derive(Default)]
struct Recorder {
    records: Vec<Value>,
}

impl EmbeddingHooks for Recorder {
    fn pre_call(&mut self, call: PreCall<'_>) {
        let mut record = Map::new();
        record.insert(String::from("phase"), json!("pre_call"));
        record.insert(String::from("input"), json!(call.input));
        record.insert(
            String::from("api_key"),
            json!(call.api_key.map(|key| key.expose_secret().to_owned())),
        );
        record.insert(String::from("body"), json!(call.body));
        if let Some(request) = call.request {
            record.insert(String::from("headers"), json!(request.headers));
            record.insert(String::from("api_base"), json!(request.api_base));
        }
        self.records.push(Value::Object(record));
    }

    fn post_call(&mut self, call: PostCall<'_>) {
        let mut record = Map::new();
        record.insert(String::from("phase"), json!("post_call"));
        record.insert(String::from("input"), json!(call.input));
        record.insert(
            String::from("api_key"),
            json!(call.api_key.map(|key| key.expose_secret().to_owned())),
        );
        record.insert(String::from("body"), json!(call.body));
        record.insert(
            String::from("original_response"),
            json!(call.original_response),
        );
        self.records.push(Value::Object(record));
    }
}

#[derive(Default)]
struct Encoder {
    encoded: Mutex<Vec<String>>,
}

impl Encoder {
    fn recorded(&self) -> Vec<String> {
        match self.encoded.lock() {
            Ok(encoded) => encoded.clone(),
            Err(_poisoned) => Vec::new(),
        }
    }
}

impl TextEncoding for Encoder {
    fn count_tokens(&self, text: &str) -> Result<usize, EngineError> {
        let Ok(mut encoded) = self.encoded.lock() else {
            return Err(EngineError::PoolClosed);
        };
        encoded.push(text.to_owned());
        Ok(text.chars().count())
    }
}

const fn mode(mode: Mode) -> SdkMode {
    match mode {
        Mode::Sync => SdkMode::Sync,
        Mode::Async => SdkMode::Async,
    }
}

fn build(input: &Call, run: &Run) -> LegacyEmbeddingPlan {
    let key = run.api_key.clone().map(SecretString::from);
    plan(&PlanInput {
        mode: mode(input.mode),
        model: &input.model,
        input: &input.input,
        optional_params: &input.optional_params,
        supplied_data: run.data.as_ref(),
        complete_api_base: run.complete_api_base.as_deref(),
        api_key: key.as_ref(),
        headers: &run.headers,
        timeout: HELPER_TIMEOUT,
    })
}

fn outcome(value: Result<LegacyEmbeddingResponse, CohereLegacyEmbeddingError>) -> Value {
    match value {
        Ok(response) => {
            let usage = response.usage;
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
    let plan = build(input, run);
    let fixture = Fixture {
        status: run.reply.status,
        body: run.reply.body.as_bytes().to_vec(),
        seen: Mutex::new(Vec::new()),
    };
    let encoder = Encoder::default();
    let mut recorder = Recorder::default();
    let key = run.api_key.clone().map(SecretString::from);
    let value = runtime.block_on(execute(
        &fixture,
        ExecuteInput {
            plan: &plan,
            input: &input.input,
            api_key: key.as_ref(),
            encoding: &encoder,
        },
        &mut recorder,
    ));
    json!({
        "url": plan.url,
        "headers": plan.headers,
        "data": plan.data,
        "hooks": recorder.records,
        "outcome": outcome(value),
        "encoded": encoder.recorded(),
    })
}

fn failure(error: &std::io::Error) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: String::from("CohereLegacyEmbedError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

impl crate::helpers::Helper for Op {
    type Input = Call;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(|error| failure(&error))?;
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
