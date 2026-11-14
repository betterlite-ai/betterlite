use std::future::{Future, ready};

use betterlite_models::decisions::Json as ModelJson;
use betterlite_models::model_capability_predicates::{CapabilityPredicates, CapabilityRequest};
use betterlite_openai::chat::response::ChatResponse;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_provider::oci_chat::config::{
    get_supported_openai_params, get_vendor_from_model, map_openai_params, validate_environment,
};
use betterlite_provider::oci_chat::payload::transform_request;
use betterlite_provider::oci_chat::response::transform_response;
use betterlite_provider::oci_chat::stream::{SseDecoder, StreamItem};
use betterlite_provider::oci_common::OciError;
use betterlite_provider::oci_common::credentials::{
    OciEnvValues, OciExplicitParams, resolve_oci_credentials,
};
use betterlite_provider::oci_common::environment::OciEnvironment;
use chrono::{DateTime, Utc};
use http::HeaderMap;
use indexmap::IndexMap;
use secrecy::SecretString;

use self::input::{ExchangeCall, Input, ResponseCall};
use crate::helpers::{HelperError, HelperResult};

mod input;

const FIXED_SECONDS: i64 = 1_730_882_977;

fn oci_failure(error: OciError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "OciError".to_owned(),
        status: Some(error.status),
        error_type: None,
        param: None,
        code: None,
        message: error.message,
    })
}

fn input_failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn text_param(params: &Object, name: &str) -> Option<String> {
    params.get(name).and_then(Json::as_str).map(str::to_owned)
}

fn explicit_params(params: &Object) -> OciExplicitParams {
    OciExplicitParams {
        oci_region: text_param(params, "oci_region"),
        oci_user: text_param(params, "oci_user"),
        oci_fingerprint: text_param(params, "oci_fingerprint"),
        oci_tenancy: text_param(params, "oci_tenancy"),
        oci_key: text_param(params, "oci_key").map(SecretString::from),
        oci_key_file: text_param(params, "oci_key_file"),
        oci_compartment_id: text_param(params, "oci_compartment_id"),
    }
}

fn supported_params(models: &[String]) -> serde_json::Value {
    let mut found = Vec::with_capacity(models.len());
    for model in models {
        found.push(serde_json::json!({
            "model": model,
            "params": get_supported_openai_params(model),
        }));
    }
    serde_json::json!({"models": found})
}

fn vendor_of(models: &[String]) -> serde_json::Value {
    let mut found = Vec::with_capacity(models.len());
    for model in models {
        found.push(serde_json::json!({
            "model": model,
            "vendor": get_vendor_from_model(model).api_format(),
        }));
    }
    serde_json::json!({"models": found})
}

fn exchange_values(credentials: &Object, calls: &[ExchangeCall]) -> HelperResult {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|source| input_failure(source.to_string()))?;
    let mut values = Vec::with_capacity(calls.len());
    for call in calls {
        values.push(match exchange_body(&runtime, credentials, call) {
            Ok(body) => serde_json::json!({"body": body}),
            Err(error) => serde_json::json!({"status": error.status}),
        });
    }
    Ok(serde_json::json!({"calls": values}))
}

fn exchange_body(
    runtime: &tokio::runtime::Runtime,
    shared: &Object,
    call: &ExchangeCall,
) -> Result<serde_json::Value, OciError> {
    let mut params = shared.clone();
    params.extend(call.params.clone());
    let credentials = resolve_oci_credentials(&explicit_params(&params), &OciEnvValues::default());
    let mut headers = HeaderMap::new();
    validate_environment(
        &mut headers,
        &call.messages,
        &credentials,
        &OciEnvironment::default(),
    )?;
    let mapped = map_openai_params(&params, &Object::new(), &call.model, call.drop_params)?;
    let mut predicates = NoCapabilities;
    let payload = runtime.block_on(transform_request(
        &call.model,
        &call.messages,
        &mapped,
        &credentials,
        &mut predicates,
    ))?;
    serde_json::to_value(&payload).map_err(|source| OciError {
        status: 500,
        message: source.to_string(),
    })
}

fn response_values(calls: &[ResponseCall]) -> HelperResult {
    let now = DateTime::<Utc>::from_timestamp(FIXED_SECONDS, 0)
        .ok_or_else(|| input_failure("the fixed date is out of range".to_owned()))?;
    let clock = FixedClock(now);
    let mut values = Vec::with_capacity(calls.len());
    for call in calls {
        let body =
            serde_json::to_vec(&call.body).map_err(|source| input_failure(source.to_string()))?;
        values.push(
            match transform_response(&call.model, call.status, &body, &IndexMap::new(), &clock) {
                Ok(reply) => project_response(&reply.response),
                Err(error) => serde_json::json!({"status": error.status}),
            },
        );
    }
    Ok(serde_json::json!({"calls": values}))
}

fn project_response(response: &ChatResponse) -> serde_json::Value {
    let choice = response.choices.first();
    let content = choice.and_then(|choice| choice.message.content.clone());
    let finish_reason = choice.and_then(|choice| choice.finish_reason.clone());
    let tool_calls = choice
        .and_then(|choice| choice.message.tool_calls.as_ref())
        .map(|calls| {
            calls
                .iter()
                .map(|call| {
                    serde_json::json!({
                        "id": call.id.clone(),
                        "name": call.function.name.clone(),
                    })
                })
                .collect::<Vec<serde_json::Value>>()
        });
    serde_json::json!({
        "model": response.model.clone(),
        "content": content,
        "finish_reason": finish_reason,
        "tool_calls": tool_calls,
        "usage": {
            "prompt_tokens": response.usage.prompt_tokens,
            "completion_tokens": response.usage.completion_tokens,
            "total_tokens": response.usage.total_tokens,
        },
    })
}

fn stream_values(reads: &[String]) -> HelperResult {
    let mut decoder = SseDecoder::default();
    let mut items = Vec::new();
    for read in reads {
        for item in decoder.push(read).map_err(oci_failure)? {
            items.push(stream_item(&item));
        }
    }
    for item in decoder.finish().map_err(oci_failure)? {
        items.push(stream_item(&item));
    }
    Ok(serde_json::json!({"items": items}))
}

fn stream_item(item: &StreamItem) -> serde_json::Value {
    match item {
        StreamItem::Done => serde_json::json!({"done": true}),
        StreamItem::Chunk(chunk) => serde_json::json!({
            "content": chunk.choice.delta.content.clone(),
            "tool_calls": chunk.tool_call,
            "finish_reason": chunk.finish_reason.clone(),
        }),
    }
}

struct NoCapabilities;

impl CapabilityPredicates for NoCapabilities {
    fn supports_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn declared_value_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = Option<String>> + Send {
        ready(None)
    }

    fn is_explicitly_disabled_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn supports_native_streaming(
        &mut self,
        _request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = ModelJson> + Send {
        ready(ModelJson::Null)
    }

    fn supports_response_schema(
        &mut self,
        _request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }
}

struct FixedClock(DateTime<Utc>);

impl betterlite_provider::oci_common::Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Supported { models } => Ok(supported_params(models)),
            Input::Vendor { models } => Ok(vendor_of(models)),
            Input::Exchange {
                credentials,
                exchanges,
            } => exchange_values(credentials, exchanges),
            Input::Response { responses } => response_values(responses),
            Input::Stream { reads } => stream_values(reads),
        }
    }
}
