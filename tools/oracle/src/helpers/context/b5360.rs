use betterlite_context::media_config_selection::{
    audio_transcription, image_edit, image_generation, text_to_speech,
};
use betterlite_context::resource_config_selection::{
    messages, passthrough, realtime, responses, search_harness, skills_evals, vector_stores,
};
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::identity::provider_id::ProviderId;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::helpers::HelperError;

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
    facts: Facts,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct Facts {
    azure: AzureFacts,
    github_copilot_responses: bool,
    mantle: MantleFacts,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct AzureFacts {
    supports_reasoning: bool,
    native_responses: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct MantleFacts {
    supported: bool,
    use_openai_path: bool,
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

const fn facts_of(facts: &Facts) -> responses::ResponsesModelFacts {
    let mantle = if facts.mantle.supported {
        responses::MantleResponses::Supported {
            use_openai_path: facts.mantle.use_openai_path,
        }
    } else {
        responses::MantleResponses::Unsupported
    };
    responses::ResponsesModelFacts {
        azure: responses::AzureResponsesFacts {
            supports_reasoning: facts.azure.supports_reasoning,
            native_responses: facts.azure.native_responses,
        },
        github_copilot_responses: facts.github_copilot_responses,
        mantle,
    }
}

fn arguments_of<T>(tag: &T) -> Value
where
    T: serde::Serialize,
{
    let serialized = serde_json::to_value(tag).unwrap_or(Value::Null);
    match serialized {
        Value::Object(fields) => fields.into_values().next().unwrap_or_else(|| json!([])),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_) => {
            json!([])
        }
    }
}

fn transcription(input: &BatchInput, call: &Call) -> Result<Value, Box<HelperError>> {
    let live = catalog(&input.model_cost)?;
    let bundled = catalog(&input.bundled_model_cost)?;
    let selected = audio_transcription::select_transcription_config(
        model_of(call)?,
        provider_of(call)?,
        &live,
        &bundled,
    );
    Ok(selected.map_or(
        Value::Null,
        |tag| json!({"class": format!("{tag:?}AudioTranscriptionConfig"), "arguments": []}),
    ))
}

fn text_to_speech(_input: &BatchInput, call: &Call) -> Result<Value, Box<HelperError>> {
    let selected =
        text_to_speech::select_text_to_speech(model_of(call)?, provider_of(call)?, call.is_lyria);
    Ok(selected.map_or(
        Value::Null,
        |tag| json!({"class": format!("{tag:?}TextToSpeechConfig"), "arguments": []}),
    ))
}

fn responses(_input: &BatchInput, call: &Call) -> Value {
    let facts = facts_of(&call.facts);
    responses::select_responses_config(call.provider, call.model.as_deref(), &facts).map_or(
        Value::Null,
        |tag| json!({"class": tag.class_name(), "arguments": arguments_of(&tag)}),
    )
}

fn vector_stores(_input: &BatchInput, call: &Call) -> Result<Value, Box<HelperError>> {
    let selected =
        vector_stores::select_vector_stores_config(provider_of(call)?, call.api_type.as_deref());
    Ok(selected.map_or(
        Value::Null,
        |tag| json!({"class": format!("{tag:?}VectorStoreConfig"), "arguments": []}),
    ))
}

fn search(_input: &BatchInput, call: &Call) -> Result<Value, Box<HelperError>> {
    let provider = call
        .search_provider
        .ok_or_else(|| invalid("a search call needs a search_provider".to_owned()))?;
    let tag = search_harness::select_search_config(provider);
    let suffix = if tag == search_harness::SearchConfigTag::SearchAPI {
        "APIConfig"
    } else {
        "SearchConfig"
    };
    Ok(json!({"class": format!("{tag:?}{suffix}"), "arguments": []}))
}

fn harness(_input: &BatchInput, call: &Call) -> Result<Value, Box<HelperError>> {
    let harness = call
        .harness
        .ok_or_else(|| invalid("a harness call needs a harness".to_owned()))?;
    let tag = search_harness::select_harness_config(harness);
    Ok(json!({"class": format!("{tag:?}HarnessConfig"), "arguments": []}))
}

fn image_generation(_input: &BatchInput, call: &Call) -> Result<Value, Box<HelperError>> {
    let selected = image_generation::select_image_generation(model_of(call)?, provider_of(call)?);
    Ok(selected.map_or(
        Value::Null,
        |tag| json!({"class": tag.class_name(), "arguments": arguments_of(&tag)}),
    ))
}

fn image_edit(_input: &BatchInput, call: &Call) -> Result<Value, Box<HelperError>> {
    let selected = image_edit::select_image_edit(model_of(call)?, provider_of(call)?);
    Ok(selected.map_or(
        Value::Null,
        |tag| json!({"class": tag.class_name(), "arguments": arguments_of(&tag)}),
    ))
}

fn messages(_input: &BatchInput, call: &Call) -> Result<Value, Box<HelperError>> {
    let selected = messages::select_messages_config(model_of(call)?, provider_of(call)?);
    Ok(selected.map_or(
        Value::Null,
        |tag| json!({"class": tag.class_name(), "arguments": arguments_of(&tag)}),
    ))
}

fn passthrough(_input: &BatchInput, call: &Call) -> Result<Value, Box<HelperError>> {
    let selected = passthrough::select_passthrough_config(model_of(call)?, provider_of(call)?);
    Ok(selected.map_or(
        Value::Null,
        |tag| json!({"class": format!("{tag:?}PassthroughConfig"), "arguments": []}),
    ))
}

fn batches(_input: &BatchInput, call: &Call) -> Result<Value, Box<HelperError>> {
    let selected = passthrough::select_batches_config(model_of(call)?, provider_of(call)?);
    Ok(selected.map_or(
        Value::Null,
        |tag| json!({"class": format!("{tag:?}BatchesConfig"), "arguments": []}),
    ))
}

fn vector_store(_input: &BatchInput, call: &Call) -> Result<Value, Box<HelperError>> {
    let selected = vector_stores::select_vector_store_config(provider_of(call)?);
    Ok(selected.map_or(
        Value::Null,
        |tag| json!({"class": format!("{tag:?}VectorStore"), "arguments": []}),
    ))
}

fn vector_store_files(_input: &BatchInput, call: &Call) -> Result<Value, Box<HelperError>> {
    let selected = vector_stores::select_store_files_config(provider_of(call)?);
    Ok(selected.map_or(
        Value::Null,
        |tag| json!({"class": format!("{tag:?}VectorStoreFilesConfig"), "arguments": []}),
    ))
}

fn realtime(_input: &BatchInput, call: &Call) -> Result<Value, Box<HelperError>> {
    let selected = realtime::select_realtime_config(model_of(call)?, provider_of(call)?);
    Ok(selected.map_or(
        Value::Null,
        |tag| json!({"class": format!("{tag:?}RealtimeConfig"), "arguments": []}),
    ))
}

fn realtime_http(_input: &BatchInput, call: &Call) -> Result<Value, Box<HelperError>> {
    let selected = realtime::select_realtime_http_config(model_of(call)?, provider_of(call)?);
    Ok(selected.map_or(
        Value::Null,
        |tag| json!({"class": format!("{tag:?}RealtimeHTTPConfig"), "arguments": []}),
    ))
}

fn skills(_input: &BatchInput, call: &Call) -> Result<Value, Box<HelperError>> {
    let selected = skills_evals::select_skills_config(provider_of(call)?);
    Ok(selected.map_or(
        Value::Null,
        |tag| json!({"class": format!("{tag:?}SkillsConfig"), "arguments": []}),
    ))
}

fn evals(_input: &BatchInput, call: &Call) -> Result<Value, Box<HelperError>> {
    let selected = skills_evals::select_evals_config(provider_of(call)?);
    Ok(selected.map_or(
        Value::Null,
        |tag| json!({"class": format!("{tag:?}EvalsConfig"), "arguments": []}),
    ))
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
        Selector::Transcription => map_calls(input, calls, transcription),
        Selector::ImageGeneration => map_calls(input, calls, image_generation),
        Selector::ImageEdit => map_calls(input, calls, image_edit),
        Selector::TextToSpeech => map_calls(input, calls, text_to_speech),
        Selector::Messages => map_calls(input, calls, messages),
        Selector::Responses => Ok(map_plain(input, calls, responses)),
        Selector::Passthrough => map_calls(input, calls, passthrough),
        Selector::Batches => map_calls(input, calls, batches),
        Selector::VectorStore => map_calls(input, calls, vector_store),
        Selector::VectorStores => map_calls(input, calls, vector_stores),
        Selector::VectorStoreFiles => map_calls(input, calls, vector_store_files),
        Selector::Realtime => map_calls(input, calls, realtime),
        Selector::RealtimeHttp => map_calls(input, calls, realtime_http),
        Selector::Search => map_calls(input, calls, search),
        Selector::Harness => map_calls(input, calls, harness),
        Selector::Skills => map_calls(input, calls, skills),
        Selector::Evals => map_calls(input, calls, evals),
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
