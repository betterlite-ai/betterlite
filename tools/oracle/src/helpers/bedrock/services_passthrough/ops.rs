use super::input::{CollectRun, Input, ReplyRun, SignRun, UrlRun};
use betterlite_bedrock::passthrough::collector::{
    DecoderChoice, StreamLogAssembler, decoder_selection,
};
use betterlite_bedrock::passthrough::reply::{
    NativeReplyRequest, NativeReplyTransform, logged_reply,
};
use betterlite_bedrock::passthrough::url::PassthroughEnvironment;
use betterlite_bedrock::passthrough::{
    BedrockPassthroughPlan, CollectorClock, PassthroughIdentity,
};
use betterlite_context::bedrock_config_selection::chat::BedrockChatConfigTag;
use betterlite_context::chat_config_selection::selection::ChatConfigError;
use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::error::ProviderError;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::passthrough::transformation::BasePassthroughConfig as _;
use betterlite_openai::passthrough::transformation::types::{
    PassthroughLogging, PassthroughSigning, PassthroughTarget, PassthroughUrlRequest,
    RelayCallType, RelayedResponse,
};
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_streaming::stream_base_assembly::provider_metadata::ChatConfigSelection;
use betterlite_streaming::stream_chunk_shaping::creation::StreamInstant;
use betterlite_streaming::stream_chunk_shaping::records::StreamChunk;
use secrecy::SecretString;
use serde_json::{Value, json};
use std::iter::once;
use std::time::SystemTime;
use uuid::Uuid;
const CLOCK: CollectorClock = CollectorClock {
    next_id: Uuid::nil,
    now: i64::default,
    instant: || StreamInstant {
        seconds: 0,
        timestamp: 0.0,
    },
};
struct NullLogging;
impl PassthroughLogging for NullLogging {
    fn set_call_type(&mut self, _call_type: RelayCallType) {}
}
struct Selector;
impl ChatConfigSelection for Selector {
    fn select_chat_config(
        &self,
        model: &str,
        provider: ProviderId,
    ) -> Result<Option<ChatConfigTag>, ChatConfigError> {
        let converse = provider == ProviderId::Bedrock && model.starts_with("converse/");
        Ok(converse.then_some(ChatConfigTag::Bedrock(BedrockChatConfigTag::Converse)))
    }
}
struct EchoTransform;
impl NativeReplyTransform for EchoTransform {
    fn transform(&self, _request: &NativeReplyRequest<'_>) -> Result<Json, Box<ProviderError>> {
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
        let content: String = chunks
            .iter()
            .flat_map(|chunk| chunk.choices.iter())
            .filter_map(|choice| choice.delta.content.as_deref())
            .collect();
        let finish_reason = chunks
            .iter()
            .filter_map(|chunk| chunk.choices.first())
            .filter_map(|choice| choice.finish_reason.as_deref())
            .next_back()
            .unwrap_or("stop");
        let view = json!({
            "model": model,
            "content": content,
            "finish_reason": finish_reason,
            "usage": chunks.iter().find_map(|chunk| chunk.usage.as_ref()).map(|usage| json!({
                "prompt_tokens": usage.prompt_tokens,
                "completion_tokens": usage.completion_tokens,
                "total_tokens": usage.total_tokens,
            })),
        });
        serde_json::from_str(&view.to_string()).ok()
    }
}
fn plan_for(input: &Input) -> BedrockPassthroughPlan<'static> {
    static SELECTOR: Selector = Selector;
    static TRANSFORM: EchoTransform = EchoTransform;
    static ASSEMBLER: ViewAssembler = ViewAssembler;
    let (environment, bearer_token) = match input {
        Input::Url { environment, .. } => (
            PassthroughEnvironment {
                region_name: environment.region_name.clone(),
                region: None,
                session_region: None,
                runtime_endpoint: environment.runtime_endpoint.clone(),
            },
            None,
        ),
        Input::Sign { bearer, .. } => (
            PassthroughEnvironment::default(),
            bearer.clone().map(SecretString::from),
        ),
        Input::Reply { .. } | Input::Collect { .. } => (PassthroughEnvironment::default(), None),
    };
    BedrockPassthroughPlan {
        environment,
        identity: PassthroughIdentity {
            credentials: None,
            bearer_token,
            now: SystemTime::UNIX_EPOCH,
        },
        selector: &SELECTOR,
        transform: &TRANSFORM,
        assembler: &ASSEMBLER,
        clock: CLOCK,
    }
}
fn url_run(plan: &BedrockPassthroughPlan<'_>, run: &UrlRun) -> HelperResult {
    let params: Object = [
        ("aws_region_name", &run.region),
        ("model_id", &run.model_id),
        ("aws_bedrock_runtime_endpoint", &run.runtime_endpoint),
    ]
    .into_iter()
    .filter_map(|(name, value)| {
        value
            .as_ref()
            .map(|text| (name.to_owned(), Value::String(text.clone())))
    })
    .collect();
    let request = PassthroughUrlRequest {
        api_base: run.api_base.as_deref(),
        api_key: None,
        target: PassthroughTarget {
            model: run.model.as_str(),
            custom_llm_provider: "bedrock",
            endpoint: run.endpoint.as_str(),
        },
        request_query_params: run.query.as_ref(),
        litellm_params: &params,
    };
    let (url, endpoint_url) = plan.get_complete_url(&request).or_value_error()?;
    Ok(json!({
        "url": url.to_string(),
        "endpoint_url": endpoint_url,
        "streaming": plan.is_streaming_request(&run.endpoint, &Object::new()),
    }))
}
fn sign_run(plan: &BedrockPassthroughPlan<'_>, run: &SignRun) -> HelperResult {
    let params: Object =
        once(("api_key".to_owned(), Value::String("sk-ignored".to_owned()))).collect();
    let context = PassthroughSigning {
        litellm_params: &params,
        request_data: run.data.as_ref(),
        api_base: "https://bedrock-runtime.us-east-1.amazonaws.com",
        model: Some("m"),
    };
    let (signed, body) = plan
        .sign_request(run.headers.clone(), &context)
        .or_value_error()?;
    let bytes = body.ok_or("the signer returned no body").or_value_error()?;
    let text = String::from_utf8(bytes).or_value_error()?;
    let pairs: Vec<Value> = signed
        .iter()
        .map(|(name, value)| json!({ "name": name, "value": value }))
        .collect();
    Ok(json!({ "headers": pairs, "body": text }))
}
fn reply_run(plan: &BedrockPassthroughPlan<'_>, run: &ReplyRun) -> Value {
    let target = PassthroughTarget {
        model: "m",
        custom_llm_provider: &run.provider,
        endpoint: &run.endpoint,
    };
    let response = RelayedResponse {
        status_code: 200,
        body: run.body.as_bytes(),
    };
    let logged = logged_reply(plan, &target, &response, &Object::new());
    json!({ "logged": matches!(logged, Ok(Some(_))), "raised": logged.is_err() })
}
fn collect_run(plan: &BedrockPassthroughPlan<'_>, run: &CollectRun, stream: &[u8]) -> HelperResult {
    let warning = match decoder_selection(&run.model, &run.endpoint, plan.clock) {
        DecoderChoice::Disabled { warning } => warning,
        DecoderChoice::Converse(_) | DecoderChoice::Invoke { .. } => None,
    };
    let target = PassthroughTarget {
        model: run.model.as_str(),
        custom_llm_provider: "bedrock",
        endpoint: run.endpoint.as_str(),
    };
    let mut collector = plan.create_stream_collector(&target);
    let (head, tail) = stream.split_at(stream.len().div_euclid(2));
    collector.add(head);
    collector.add(tail);
    let mut logging = NullLogging;
    let view = collector
        .build_logged_response(&mut logging)
        .or_value_error()?;
    Ok(json!({
        "decoded": view.is_some(),
        "view": view,
        "warning": warning,
    }))
}
pub(super) fn run(input: &Input) -> HelperResult {
    let plan = plan_for(input);
    let values = match input {
        Input::Url { runs, .. } => runs
            .iter()
            .map(|run| url_run(&plan, run))
            .collect::<Result<Vec<_>, _>>()?,
        Input::Sign { runs, .. } => runs
            .iter()
            .map(|run| sign_run(&plan, run))
            .collect::<Result<Vec<_>, _>>()?,
        Input::Reply { runs } => runs.iter().map(|run| reply_run(&plan, run)).collect(),
        Input::Collect { runs, stream } => {
            let bytes = hex::decode(stream).or_value_error()?;
            runs.iter()
                .map(|run| collect_run(&plan, run, &bytes))
                .collect::<Result<Vec<_>, _>>()?
        }
    };
    Ok(json!({ "runs": values }))
}
