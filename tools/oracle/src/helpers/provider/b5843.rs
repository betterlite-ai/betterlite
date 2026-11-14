use std::future::{Future, ready};

use betterlite_models::decisions::Json as ModelJson;
use betterlite_models::model_capability_predicates::{CapabilityPredicates, CapabilityRequest};
use betterlite_prompts::arguments::json::Object;
use betterlite_provider::oci_chat::config::{
    get_supported_openai_params, get_vendor_from_model, map_openai_params, validate_environment,
};
use betterlite_provider::oci_chat::payload::{OciCompletionPayload, transform_request};
use betterlite_provider::oci_chat::response::{ResponseInput, transform_response};
use betterlite_provider::oci_chat::stream::{SseDecoder, StreamItem};
use betterlite_provider::oci_common::credentials::{
    OciEnvValues, OciExplicitParams, resolve_oci_credentials,
};
use betterlite_provider::oci_common::environment::OciEnvironment;
use betterlite_provider::oci_common::{Clock, OciError};
use betterlite_provider::ollama_chat::ids::IdSource;
use chrono::{DateTime, Utc};
use http::HeaderMap;
use indexmap::IndexMap;
use serde_json::json;

use self::input::{ExchangeCall, Input, ResponseCall};
use crate::helpers::{HelperResult, OrHelperError as _};

mod input;

const FIXED_SECONDS: i64 = 1_730_882_977;

fn per_model(models: &[String], answer: fn(&str) -> serde_json::Value) -> serde_json::Value {
    let mut found = Vec::with_capacity(models.len());
    for model in models {
        found.push(answer(model));
    }
    json!({"models": found})
}

fn exchange_values(shared: &Object, calls: &[ExchangeCall]) -> HelperResult {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .or_value_error()?;
    let mut values = Vec::with_capacity(calls.len());
    for call in calls {
        let mut params = shared.clone();
        params.extend(call.params.clone());
        let explicit: OciExplicitParams =
            serde_json::from_slice(&serde_json::to_vec(&params).or_value_error()?)
                .or_value_error()?;
        values.push(match exchange_payload(&runtime, call, &params, &explicit) {
            Ok(payload) => json!({"body": serde_json::to_value(&payload).or_value_error()?}),
            Err(error) => json!({"status": error.status}),
        });
    }
    Ok(json!({"calls": values}))
}

fn exchange_payload(
    runtime: &tokio::runtime::Runtime,
    call: &ExchangeCall,
    params: &Object,
    explicit: &OciExplicitParams,
) -> Result<OciCompletionPayload, OciError> {
    let credentials = resolve_oci_credentials(explicit, &OciEnvValues::default());
    validate_environment(
        &mut HeaderMap::new(),
        &call.messages,
        &credentials,
        &OciEnvironment::default(),
    )?;
    let mapped = map_openai_params(params, &Object::new(), &call.model, call.drop_params)?;
    runtime.block_on(transform_request(
        &call.model,
        &call.messages,
        &mapped,
        &credentials,
        &mut NoCapabilities,
    ))
}

fn response_values(calls: &[ResponseCall]) -> HelperResult {
    let headers = IndexMap::new();
    let mut values = Vec::with_capacity(calls.len());
    for call in calls {
        let body = serde_json::to_vec(&call.body).or_value_error()?;
        let input = ResponseInput {
            model: &call.model,
            status: call.status,
            body: &body,
            headers: &headers,
        };
        values.push(match transform_response(input, &FixedClock, &FixedIds) {
            Ok(reply) => {
                let choice = reply.response.choices.first();
                json!({
                    "model": reply.response.model,
                    "content": choice.and_then(|found| found.message.content.clone()),
                    "finish_reason": choice.and_then(|found| found.finish_reason.clone()),
                })
            }
            Err(error) => json!({"status": error.status}),
        });
    }
    Ok(json!({"calls": values}))
}

fn stream_values(reads: &[String]) -> HelperResult {
    let mut decoder = SseDecoder::default();
    let mut events = Vec::new();
    for read in reads {
        events.extend(decoder.push(read).or_value_error()?);
    }
    events.extend(decoder.finish().or_value_error()?);
    let mut items = Vec::with_capacity(events.len());
    for item in &events {
        items.push(match item {
            StreamItem::Done => json!({"done": true}),
            StreamItem::Chunk(chunk) => json!({
                "content": chunk.choice.delta.content.clone(),
                "tool_calls": chunk.tool_call,
                "finish_reason": chunk.finish_reason.clone(),
            }),
        });
    }
    Ok(json!({"items": items}))
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

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        DateTime::from_timestamp(FIXED_SECONDS, 0).unwrap_or_default()
    }
}

struct FixedIds;

impl IdSource for FixedIds {
    fn next_id(&self) -> String {
        "fixed".to_owned()
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Supported { models } => Ok(per_model(
                models,
                |model| json!({"model": model, "params": get_supported_openai_params(model)}),
            )),
            Input::Vendor { models } => Ok(per_model(
                models,
                |model| json!({"model": model, "vendor": get_vendor_from_model(model).api_format()}),
            )),
            Input::Exchange {
                credentials,
                exchanges,
            } => exchange_values(credentials, exchanges),
            Input::Response { responses } => response_values(responses),
            Input::Stream { reads } => stream_values(reads),
        }
    }
}
