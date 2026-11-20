use betterlite_bedrock::passthrough::collector::StreamLogAssembler;
use betterlite_bedrock::passthrough::reply::{NativeReplyRequest, NativeReplyTransform};
use betterlite_bedrock::passthrough::url::PassthroughEnvironment;
use betterlite_bedrock::passthrough::{
    BedrockPassthroughPlan, CollectorClock, PassthroughIdentity,
};
use betterlite_context::chat_config_selection::selection::ChatConfigError;
use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::error::ProviderError;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::passthrough::transformation::BasePassthroughConfig as _;
use betterlite_openai::passthrough::transformation::types::{
    PassthroughLogging, PassthroughSigning, PassthroughTarget, PassthroughUrlRequest, RelayCallType,
};
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_provider::bedrock_interactive_dispatch::passthrough;
use betterlite_streaming::stream_base_assembly::provider_metadata::ChatConfigSelection;
use betterlite_streaming::stream_chunk_shaping::creation::StreamInstant;
use betterlite_streaming::stream_chunk_shaping::records::StreamChunk;
use http::Method;

use super::{PassthroughRun, shared};

struct Selector;

impl ChatConfigSelection for Selector {
    fn select_chat_config(
        &self,
        _model: &str,
        _provider: ProviderId,
    ) -> Result<Option<ChatConfigTag>, ChatConfigError> {
        Ok(Some(ChatConfigTag::Anthropic))
    }
}

// The reply the plan would transform, echoed so both adapters compare what the hook was handed
struct Transform;

impl NativeReplyTransform for Transform {
    fn transform(&self, request: &NativeReplyRequest<'_>) -> Result<Json, Box<ProviderError>> {
        let reply = shared::json!({
            "status_code": request.status_code,
            "body": String::from_utf8_lossy(request.body),
            "request_data": request.request_data
        });
        Ok(serde_json::from_value(reply).unwrap_or(Json::Null))
    }
}

// The decoded stream as the logged reply: its assistant text and the stop reason it ended on
struct Assembler;

impl StreamLogAssembler for Assembler {
    fn assemble(
        &self,
        chunks: &[StreamChunk],
        _logging_obj: &mut dyn PassthroughLogging,
    ) -> Option<Json> {
        if chunks.is_empty() {
            return None;
        }
        let content: String = chunks
            .iter()
            .flat_map(|chunk| chunk.choices.iter())
            .filter_map(|choice| choice.delta.content.as_deref())
            .collect();
        let finish = chunks
            .iter()
            .flat_map(|chunk| chunk.choices.iter())
            .find_map(|choice| choice.finish_reason.clone());
        let view = shared::json!({"content": content, "finish_reason": finish});
        Some(serde_json::from_value(view).unwrap_or(Json::Null))
    }
}

struct Logging;

impl PassthroughLogging for Logging {
    fn set_call_type(&mut self, _call_type: RelayCallType) {}
}

struct PassthroughServices {
    selector: Selector,
    transform: Transform,
    assembler: Assembler,
    params: Object,
    data: Option<Object>,
}

impl PassthroughServices {
    fn new(run: &PassthroughRun) -> Result<Self, Box<shared::HelperError>> {
        Ok(Self {
            selector: Selector,
            transform: Transform,
            assembler: Assembler,
            params: Object::new(),
            data: run
                .data
                .clone()
                .map(serde_json::from_value::<Object>)
                .transpose()?,
        })
    }

    fn plan<'call>(&'call self, run: &'call PassthroughRun) -> BedrockPassthroughPlan<'call> {
        BedrockPassthroughPlan {
            environment: PassthroughEnvironment {
                region_name: Some(run.region.clone().unwrap_or_else(|| "us-east-1".to_owned())),
                region: None,
                session_region: None,
                runtime_endpoint: None,
            },
            identity: PassthroughIdentity {
                credentials: Some(shared::credentials()),
                bearer_token: None,
                now: shared::fixture_time(),
            },
            selector: &self.selector,
            transform: &self.transform,
            assembler: &self.assembler,
            clock: CollectorClock {
                next_id: || uuid::Uuid::from_u128(7),
                now: || 1_767_225_600,
                instant: || StreamInstant {
                    seconds: 1_767_225_600,
                    timestamp: 1_767_225_600.0,
                },
            },
        }
    }
}

fn passthrough_url_request<'call>(
    params: &'call Object,
    endpoint: &'call str,
) -> PassthroughUrlRequest<'call> {
    PassthroughUrlRequest {
        api_base: None,
        api_key: None,
        target: PassthroughTarget {
            model: shared::PASSTHROUGH_MODEL,
            custom_llm_provider: "bedrock",
            endpoint,
        },
        request_query_params: None,
        litellm_params: params,
    }
}

pub(crate) fn passthrough_row(run: &PassthroughRun) -> shared::HelperResult {
    let services = PassthroughServices::new(run)?;
    let method_name = run.method.as_deref().unwrap_or("POST");
    let method = Method::from_bytes(method_name.as_bytes()).or_value_error()?;
    let plan = services.plan(run);
    let url_request = passthrough_url_request(&services.params, &run.endpoint);
    let Ok((url, endpoint_url)) = plan.get_complete_url(&url_request) else {
        return Ok(shared::error_row("leaf", None));
    };
    let signed_base = url.to_string();
    let signing = PassthroughSigning {
        litellm_params: &services.params,
        request_data: services.data.as_ref(),
        api_base: &signed_base,
        model: Some(shared::PASSTHROUGH_MODEL),
    };
    let mut logging = Logging;
    let operation = passthrough::Operation {
        plan,
        url_request,
        signing,
        headers: Headers::new(),
        method,
        deadline: shared::DEADLINE,
        logging: &mut logging,
    };
    let mut transport = shared::Scripted(shared::reply(run.status, &run.reply));
    for chunk in &run.chunks {
        transport
            .0
            .chunks
            .push(hex::decode(chunk).or_value_error()?);
    }
    match shared::block_on(operation.run(&transport))? {
        Ok(outcome) => success(&outcome, &endpoint_url),
        Err(error) => Ok(failure(&error)),
    }
}

fn success(outcome: &passthrough::Outcome, endpoint_url: &str) -> shared::HelperResult {
    let mut row = shared::success_row(&outcome.request);
    row.insert("endpoint_url".to_owned(), shared::json!(endpoint_url));
    row.insert("streaming".to_owned(), shared::json!(outcome.streaming));
    row.insert("status".to_owned(), shared::json!(outcome.status));
    row.insert(
        "logged".to_owned(),
        serde_json::to_value(&outcome.logged).or_value_error()?,
    );
    Ok(shared::Value::Object(row))
}

fn failure(error: &passthrough::Error) -> shared::Value {
    match error {
        passthrough::Error::Leaf(_) => shared::error_row("leaf", None),
        passthrough::Error::Transport(_) => shared::error_row("transport", None),
        passthrough::Error::Collector { .. } => shared::error_row("collector", None),
    }
}
