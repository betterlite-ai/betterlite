use betterlite_context::resource_config_selection::{responses, search_harness};
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::identity::provider_id::ProviderId;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::helpers::HelperError;

mod selectors;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct BatchInput {
    model_cost: Map<String, Value>,
    bundled_model_cost: Map<String, Value>,
    requests: Vec<Request>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    selector: Selector,
    calls: Vec<Call>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Selector {
    Transcription,
    ImageGeneration,
    ImageEdit,
    TextToSpeech,
    Messages,
    Responses,
    Passthrough,
    Batches,
    VectorStore,
    VectorStores,
    VectorStoreFiles,
    Realtime,
    RealtimeHttp,
    Search,
    Harness,
    Skills,
    Evals,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct Call {
    model: Option<String>,
    provider: Option<ProviderId>,
    api_type: Option<String>,
    search_provider: Option<search_harness::SearchProvider>,
    harness: Option<search_harness::Harness>,
    is_lyria: bool,
    facts: Value,
}

fn invalid(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn catalog(entries: &Map<String, Value>) -> Result<ModelCatalog, Box<HelperError>> {
    let bytes = serde_json::to_vec(entries).map_err(|error| invalid(error.to_string()))?;
    let parsed = ModelCatalog::parse(&bytes, UnknownFields::Warn);
    parsed.map_err(|error| invalid(error.to_string()))
}

fn provider_of(call: &Call) -> Result<ProviderId, Box<HelperError>> {
    call.provider
        .ok_or_else(|| invalid("a call needs a provider".to_owned()))
}

fn model_of(call: &Call) -> Result<&str, Box<HelperError>> {
    call.model
        .as_deref()
        .ok_or_else(|| invalid("a call needs a model".to_owned()))
}

fn flag(fields: Option<&Value>, name: &str) -> bool {
    let value = fields.and_then(|object| object.get(name));
    let flag = value.and_then(Value::as_bool);
    flag.unwrap_or(false)
}

fn facts_of(facts: &Value) -> responses::ResponsesModelFacts {
    let azure = facts.get("azure");
    let mantle = facts.get("mantle");
    let supported = if flag(mantle, "supported") {
        responses::MantleResponses::Supported {
            use_openai_path: flag(mantle, "use_openai_path"),
        }
    } else {
        responses::MantleResponses::Unsupported
    };
    responses::ResponsesModelFacts {
        azure: responses::AzureResponsesFacts {
            supports_reasoning: flag(azure, "supports_reasoning"),
            native_responses: flag(azure, "native_responses"),
        },
        github_copilot_responses: flag(Some(facts), "github_copilot_responses"),
        mantle: supported,
    }
}

type CallSelect = fn(&BatchInput, &Call) -> Result<Value, Box<HelperError>>;

fn map_calls(
    input: &BatchInput,
    calls: &[Call],
    select: CallSelect,
) -> Result<Vec<Value>, Box<HelperError>> {
    let mut results = Vec::with_capacity(calls.len());
    for call in calls {
        results.push(select(input, call)?);
    }
    Ok(results)
}

fn map_plain(
    input: &BatchInput,
    calls: &[Call],
    select: fn(&BatchInput, &Call) -> Value,
) -> Vec<Value> {
    let mut results = Vec::with_capacity(calls.len());
    for call in calls {
        results.push(select(input, call));
    }
    results
}

fn run_request(
    input: &BatchInput,
    selector: Selector,
    calls: &[Call],
) -> Result<Vec<Value>, Box<HelperError>> {
    match selector {
        Selector::Transcription => map_calls(input, calls, selectors::transcription),
        Selector::ImageGeneration => map_calls(input, calls, selectors::image_generation),
        Selector::ImageEdit => map_calls(input, calls, selectors::image_edit),
        Selector::TextToSpeech => map_calls(input, calls, selectors::text_to_speech),
        Selector::Messages => map_calls(input, calls, selectors::messages),
        Selector::Responses => Ok(map_plain(input, calls, selectors::responses)),
        Selector::Passthrough => map_calls(input, calls, selectors::passthrough),
        Selector::Batches => map_calls(input, calls, selectors::batches),
        Selector::VectorStore => map_calls(input, calls, selectors::vector_store),
        Selector::VectorStores => map_calls(input, calls, selectors::vector_stores),
        Selector::VectorStoreFiles => map_calls(input, calls, selectors::vector_store_files),
        Selector::Realtime => map_calls(input, calls, selectors::realtime),
        Selector::RealtimeHttp => map_calls(input, calls, selectors::realtime_http),
        Selector::Search => map_calls(input, calls, selectors::search),
        Selector::Harness => map_calls(input, calls, selectors::harness),
        Selector::Skills => map_calls(input, calls, selectors::skills),
        Selector::Evals => map_calls(input, calls, selectors::evals),
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = BatchInput;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        input
            .requests
            .iter()
            .map(|request| run_request(input, request.selector, &request.calls).map(Value::Array))
            .collect::<Result<Vec<Value>, Box<HelperError>>>()
            .map(Value::Array)
    }
}
