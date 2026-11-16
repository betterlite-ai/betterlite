use super::input::{Environment, Frame, Input, ReplyRun, SelectionRun, SignRun, UrlRun};
use betterlite_bedrock::passthrough::BedrockPassthroughPlan;
use betterlite_bedrock::passthrough::CollectorClock;
use betterlite_bedrock::passthrough::PassthroughIdentity;
use betterlite_bedrock::passthrough::collector::{
    DecoderChoice, StreamLogAssembler, decoder_selection,
};
use betterlite_bedrock::passthrough::reply::{NativeReplyRequest, NativeReplyTransform};
use betterlite_bedrock::passthrough::url::PassthroughEnvironment;
use betterlite_context::chat_config_selection::selection::ChatConfigError;
use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::error::ProviderError;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::passthrough::transformation::types::{
    PassthroughLogging, PassthroughSigning, PassthroughTarget, PassthroughUrlRequest,
    RelayCallType, RelayedResponse,
};
use betterlite_openai::passthrough::transformation::{
    BasePassthroughConfig, PassthroughStreamCollector,
};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_streaming::stream_base_assembly::provider_metadata::ChatConfigSelection;
use betterlite_streaming::stream_chunk_shaping::creation::StreamInstant;
use betterlite_streaming::stream_chunk_shaping::records::StreamChunk;
use secrecy::SecretString;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fmt::Display;
use std::time::SystemTime;
use uuid::Uuid;
fn failure<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "BedrockPassthrough".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
fn object(pairs: &[(&str, Value)]) -> Object {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}
fn strings(map: &BTreeMap<String, String>) -> Object {
    map.iter()
        .map(|(name, value)| (name.clone(), Value::String(value.clone())))
        .collect()
}
fn environment_of(source: &Environment) -> PassthroughEnvironment {
    PassthroughEnvironment {
        region_name: source.region_name.clone(),
        region: None,
        session_region: None,
        runtime_endpoint: source.runtime_endpoint.clone(),
    }
}
fn identity(bearer: Option<String>) -> PassthroughIdentity {
    PassthroughIdentity {
        credentials: None,
        bearer_token: bearer.map(SecretString::from),
        now: SystemTime::UNIX_EPOCH,
    }
}
fn fixed_id() -> Uuid {
    Uuid::nil()
}
fn fixed_now() -> i64 {
    0
}
fn fixed_instant() -> StreamInstant {
    StreamInstant {
        seconds: 0,
        timestamp: 0.0,
    }
}
const CLOCK: CollectorClock = CollectorClock {
    next_id: fixed_id,
    now: fixed_now,
    instant: fixed_instant,
};
struct NullLogging;
impl PassthroughLogging for NullLogging {
    fn set_call_type(&mut self, _call_type: RelayCallType) {}
}
struct NoConfig;
impl ChatConfigSelection for NoConfig {
    fn select_chat_config(
        &self,
        _model: &str,
        _provider: ProviderId,
    ) -> Result<Option<ChatConfigTag>, ChatConfigError> {
        Ok(None)
    }
}
struct EchoTransform;
impl NativeReplyTransform for EchoTransform {
    fn transform(&self, _request: &NativeReplyRequest<'_>) -> Result<Json, ProviderError> {
        Ok(Json::Null)
    }
}
struct ViewAssembler;
impl StreamLogAssembler for ViewAssembler {
    fn assemble(
        &self,
        chunks: &[StreamChunk],
        _logging_obj: &mut dyn PassthroughLogging,
    ) -> Option<Json> {
        let first = chunks.first()?;
        let model = chunks
            .iter()
            .find(|chunk| !chunk.model.is_empty() && chunk.model != first.model)
            .map_or_else(|| first.model.clone(), |chunk| chunk.model.clone());
        let role = chunks
            .iter()
            .find_map(|chunk| chunk.choices.first())
            .and_then(|choice| choice.delta.role.clone())
            .unwrap_or_else(|| "assistant".to_owned());
        let content: String = chunks
            .iter()
            .flat_map(|chunk| chunk.choices.iter())
            .filter_map(|choice| choice.delta.content.clone())
            .collect();
        let finish_reason = chunks
            .iter()
            .flat_map(|chunk| chunk.choices.iter())
            .filter_map(|choice| choice.finish_reason.clone())
            .last()
            .unwrap_or_else(|| "stop".to_owned());
        let usage = chunks
            .iter()
            .find_map(|chunk| chunk.usage.as_ref())
            .map(|usage| {
                json!({
                    "prompt_tokens": usage.prompt_tokens,
                    "completion_tokens": usage.completion_tokens,
                    "total_tokens": usage.total_tokens,
                })
            });
        let view = json!({
            "model": model,
            "role": role,
            "content": content,
            "finish_reason": finish_reason,
            "usage": usage,
        });
        serde_json::from_str(&view.to_string()).ok()
    }
}
fn url_params(run: &UrlRun) -> Object {
    let mut params = Object::new();
    for (name, value) in [
        ("aws_region_name", run.region.as_ref()),
        ("model_id", run.model_id.as_ref()),
        (
            "aws_bedrock_runtime_endpoint",
            run.runtime_endpoint.as_ref(),
        ),
    ] {
        if let Some(text) = value {
            params.insert(name.to_owned(), Value::String(text.clone()));
        }
    }
    params
}
fn url_run(plan: &BedrockPassthroughPlan<'_>, run: &UrlRun) -> Result<Value, Box<HelperError>> {
    let params = url_params(run);
    let query = run.query.as_ref().map(strings);
    let body_flag = object(&[("stream", Value::Bool(true))]);
    let target = PassthroughTarget {
        model: &run.model,
        custom_llm_provider: "bedrock",
        endpoint: &run.endpoint,
    };
    let request = PassthroughUrlRequest {
        api_base: run.api_base.as_deref(),
        api_key: None,
        target,
        request_query_params: query.as_ref(),
        litellm_params: &params,
    };
    let (url, endpoint_url) = plan.get_complete_url(&request).map_err(failure)?;
    let streaming = plan.is_streaming_request(&run.endpoint, &body_flag);
    Ok(json!({
        "url": url.to_string(),
        "endpoint_url": endpoint_url,
        "streaming": streaming,
    }))
}
fn url_runs(environment: &Environment, runs: &[UrlRun]) -> HelperResult {
    let selector = NoConfig;
    let transform = EchoTransform;
    let assembler = ViewAssembler;
    let plan = BedrockPassthroughPlan {
        environment: environment_of(environment),
        identity: identity(None),
        selector: &selector,
        transform: &transform,
        assembler: &assembler,
        clock: CLOCK,
    };
    let mut values = Vec::with_capacity(runs.len());
    for run in runs {
        values.push(url_run(&plan, run)?);
    }
    Ok(json!({ "runs": values }))
}
fn sign_plan(bearer: Option<String>) -> BedrockPassthroughPlan<'static> {
    static SELECTOR: NoConfig = NoConfig;
    static TRANSFORM: EchoTransform = EchoTransform;
    static ASSEMBLER: ViewAssembler = ViewAssembler;
    BedrockPassthroughPlan {
        environment: PassthroughEnvironment::default(),
        identity: identity(bearer),
        selector: &SELECTOR,
        transform: &TRANSFORM,
        assembler: &ASSEMBLER,
        clock: CLOCK,
    }
}
fn request_object(run: &SignRun) -> Result<Option<Object>, Box<HelperError>> {
    match run.data.as_ref() {
        None => Ok(None),
        Some(Value::Object(data)) => Ok(Some(data.clone())),
        Some(_other) => Err(failure("the sign data is not an object")),
    }
}
fn sign_run(plan: &BedrockPassthroughPlan<'_>, run: &SignRun) -> Result<Value, Box<HelperError>> {
    let params = object(&[("api_key", Value::String("sk-ignored".to_owned()))]);
    let data = request_object(run)?;
    let context = PassthroughSigning {
        litellm_params: &params,
        request_data: data.as_ref(),
        api_base: "https://bedrock-runtime.us-east-1.amazonaws.com",
        model: Some("m"),
    };
    let headers: Headers = run
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let (signed, body) = plan.sign_request(headers, &context).map_err(failure)?;
    let body = body.ok_or_else(|| failure("the signer returned no body"))?;
    let text = String::from_utf8(body).map_err(failure)?;
    let pairs: Vec<Value> = signed
        .iter()
        .map(|(name, value)| json!({ "name": name, "value": value }))
        .collect();
    Ok(json!({ "headers": pairs, "body": text }))
}
fn sign_runs(bearer: Option<String>, runs: &[SignRun]) -> HelperResult {
    let plan = sign_plan(bearer);
    let mut values = Vec::with_capacity(runs.len());
    for run in runs {
        values.push(sign_run(&plan, run)?);
    }
    Ok(json!({ "runs": values }))
}
fn reply_run(plan: &BedrockPassthroughPlan<'_>, run: &ReplyRun) -> Value {
    let target = PassthroughTarget {
        model: "m",
        custom_llm_provider: &run.provider,
        endpoint: &run.endpoint,
    };
    let response = RelayedResponse {
        status_code: 200,
        body: b"{}",
    };
    let request_data = Object::new();
    match betterlite_bedrock::passthrough::reply::logged_reply(
        plan,
        &target,
        &response,
        &request_data,
    ) {
        Ok(logged) => json!({ "logged": logged, "raised": false }),
        Err(_error) => json!({ "logged": null, "raised": true }),
    }
}
fn reply_runs(runs: &[ReplyRun]) -> HelperResult {
    let selector = NoConfig;
    let transform = EchoTransform;
    let assembler = ViewAssembler;
    let plan = BedrockPassthroughPlan {
        environment: PassthroughEnvironment::default(),
        identity: identity(None),
        selector: &selector,
        transform: &transform,
        assembler: &assembler,
        clock: CLOCK,
    };
    let values: Vec<Value> = runs.iter().map(|run| reply_run(&plan, run)).collect();
    Ok(json!({ "runs": values }))
}
fn collector_plan() -> BedrockPassthroughPlan<'static> {
    static SELECTOR: NoConfig = NoConfig;
    static TRANSFORM: EchoTransform = EchoTransform;
    static ASSEMBLER: ViewAssembler = ViewAssembler;
    BedrockPassthroughPlan {
        environment: PassthroughEnvironment::default(),
        identity: identity(None),
        selector: &SELECTOR,
        transform: &TRANSFORM,
        assembler: &ASSEMBLER,
        clock: CLOCK,
    }
}
fn feed(collector: &mut dyn PassthroughStreamCollector<Logged = Json>, bytes: &[u8], split: bool) {
    let midpoint = bytes.len() / 2;
    if split && midpoint > 0 {
        collector.add(&bytes[..midpoint]);
        collector.add(&bytes[midpoint..]);
    } else {
        collector.add(bytes);
    }
}
fn collect(
    plan: &BedrockPassthroughPlan<'_>,
    model: &str,
    endpoint: &str,
    frames: &[Frame],
    split: bool,
) -> Result<Option<Json>, Box<HelperError>> {
    let target = PassthroughTarget {
        model,
        custom_llm_provider: "bedrock",
        endpoint,
    };
    let bytes = frames_bytes(frames)?;
    let mut collector = plan.create_stream_collector(&target);
    feed(collector.as_mut(), &bytes, split);
    let mut logging = NullLogging;
    collector
        .build_logged_response(&mut logging)
        .map_err(failure)
}
fn collector_runs(model: &str, endpoint: &str, split: bool, frames: &[Frame]) -> HelperResult {
    let plan = collector_plan();
    let logged = collect(&plan, model, endpoint, frames, split)?;
    Ok(json!({ "runs": [{ "decoded": logged.is_some(), "view": logged }] }))
}
fn selection_run(
    plan: &BedrockPassthroughPlan<'_>,
    run: &SelectionRun,
    frames: &[Frame],
) -> Result<Value, Box<HelperError>> {
    let (warning, enabled) = match decoder_selection(&run.model, &run.endpoint, CLOCK) {
        DecoderChoice::Disabled { warning } => (warning, false),
        DecoderChoice::Converse(_) | DecoderChoice::Invoke { .. } => (None, true),
    };
    let decoded = enabled && collect(plan, &run.model, &run.endpoint, frames, false)?.is_some();
    Ok(json!({ "decoded": decoded, "warning": warning }))
}
fn selection_runs(runs: &[SelectionRun], frames: &[Frame]) -> HelperResult {
    let plan = collector_plan();
    let mut values = Vec::with_capacity(runs.len());
    for run in runs {
        values.push(selection_run(&plan, run, frames)?);
    }
    Ok(json!({ "runs": values }))
}
fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _bit in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}
fn header_bytes(name: &str, value: &str) -> Result<Vec<u8>, Box<HelperError>> {
    let name_len = u8::try_from(name.len()).map_err(failure)?;
    let value_len = u16::try_from(value.len()).map_err(failure)?;
    let mut bytes = Vec::with_capacity(name.len() + value.len() + 4);
    bytes.push(name_len);
    bytes.extend_from_slice(name.as_bytes());
    bytes.push(7);
    bytes.extend_from_slice(&value_len.to_be_bytes());
    bytes.extend_from_slice(value.as_bytes());
    Ok(bytes)
}
fn encode_frame(message: &Frame) -> Result<Vec<u8>, Box<HelperError>> {
    let mut headers = Vec::new();
    for (name, value) in [
        (":event-type", message.event_type.as_str()),
        (":content-type", "application/json"),
        (":message-type", "event"),
    ] {
        headers.extend_from_slice(&header_bytes(name, value)?);
    }
    let payload = message.payload.as_bytes();
    let total = u32::try_from(headers.len() + payload.len() + 16).map_err(failure)?;
    let headers_len = u32::try_from(headers.len()).map_err(failure)?;
    let mut prelude = total.to_be_bytes().to_vec();
    prelude.extend_from_slice(&headers_len.to_be_bytes());
    let mut bytes = prelude.clone();
    bytes.extend_from_slice(&crc32(&prelude).to_be_bytes());
    bytes.extend_from_slice(&headers);
    bytes.extend_from_slice(payload);
    let checksum = crc32(&bytes);
    bytes.extend_from_slice(&checksum.to_be_bytes());
    Ok(bytes)
}
fn frames_bytes(frames: &[Frame]) -> Result<Vec<u8>, Box<HelperError>> {
    let mut bytes = Vec::new();
    for message in frames {
        bytes.extend_from_slice(&encode_frame(message)?);
    }
    Ok(bytes)
}
pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::Url { environment, runs } => url_runs(environment, runs),
        Input::Sign { bearer, runs } => sign_runs(bearer.clone(), runs),
        Input::Reply { runs } => reply_runs(runs),
        Input::Collector {
            model,
            endpoint,
            split,
            frames,
        } => collector_runs(model, endpoint, *split, frames),
        Input::Selection { runs, frames } => selection_runs(runs, frames),
    }
}
