use std::collections::{BTreeMap, VecDeque};
use std::fmt::Display;
use std::future::{Future, ready};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_core::api::runway::images::cost::cost_calculator;
use betterlite_core::api::runway::images::error::RunwayImageError;
use betterlite_core::api::runway::images::params::{map_openai_params, request_body};
use betterlite_core::api::runway::images::transform::RunwayImageTransform;
use betterlite_core::api::runway::images::{RunwayImageConfig, SUPPORTED_OPENAI_PARAMS};
use betterlite_core::api::runway::tasks::error::RunwayTaskError;
use betterlite_core::api::runway::tasks::poll::{PollClock, TaskTransport};
use betterlite_core::cost::image_dispatch::policy::{ImageResponseView, PricedImage};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::image_cost::ImageBilling;
use betterlite_models::image_cost::error::ImageCostError;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_openai::api::image_contracts::records::{
    ImageData, ImageHeaders, ImageHttpResponse, ImageParams, ImageResponse,
};
use reqwest::header::{AUTHORIZATION, HeaderMap};
use serde::Deserialize;
use serde_json::{Value, json};
use url::Url;

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MappingCall {
    #[serde(default)]
    non_default_params: ImageParams,
    #[serde(default)]
    optional_params: ImageParams,
    #[serde(default)]
    drop_params: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Mapping {
    model: String,
    prompt: String,
    calls: Vec<MappingCall>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Submission {
    status: u16,
    #[serde(default)]
    headers: ImageHeaders,
    body: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    status: u16,
    body: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResponseInput {
    model: String,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    submission: Submission,
    #[serde(default)]
    request_headers: ImageHeaders,
    #[serde(default)]
    created: i64,
    #[serde(default)]
    data: Vec<ImageData>,
    #[serde(default)]
    step_millis: u64,
    replies: Vec<Reply>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CostCall {
    #[serde(default)]
    data: Vec<ImageData>,
    prices: PriceRecord,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cost {
    model: String,
    calls: Vec<CostCall>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", content = "params", rename_all = "snake_case")]
pub(crate) enum Input {
    Mapping(Mapping),
    Response(ResponseInput),
    Cost(Cost),
}

struct Script {
    replies: Mutex<VecDeque<(u16, String)>>,
    requests: Mutex<Vec<(String, HeaderMap)>>,
}

impl Script {
    fn new(replies: &[Reply]) -> Self {
        Self {
            replies: Mutex::new(
                replies
                    .iter()
                    .map(|reply| (reply.status, reply.body.clone()))
                    .collect(),
            ),
            requests: Mutex::default(),
        }
    }

    fn describe(&self) -> Vec<Value> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|(url, headers)| {
                json!({
                    "url": url,
                    "headers": {
                        "authorization": header_text(headers, AUTHORIZATION.as_str()),
                        "x-runway-version": header_text(headers, "x-runway-version"),
                    },
                })
            })
            .collect()
    }
}

impl TaskTransport for Script {
    fn get(
        &self,
        url: Url,
        headers: HeaderMap,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((url.to_string(), headers));
        let (status, body) = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or((500, String::new()));
        ready(Ok(UpstreamResponse {
            status,
            headers: HeaderMap::new(),
            body: body.into_bytes(),
        }))
    }
}

fn header_text(headers: &HeaderMap, name: &str) -> String {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

struct Clock {
    millis: AtomicU64,
    step: u64,
    pauses: Mutex<Vec<u64>>,
}

impl Clock {
    fn new(step: u64) -> Self {
        Self {
            millis: AtomicU64::new(0),
            step,
            pauses: Mutex::default(),
        }
    }
}

impl PollClock for Clock {
    fn now(&self) -> Duration {
        Duration::from_millis(self.millis.fetch_add(self.step, Ordering::SeqCst))
    }

    fn pause(&self, duration: Duration) -> impl Future<Output = ()> + Send {
        self.pauses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(duration.as_secs());
        ready(())
    }
}

struct EmptyLookup;

impl PriceLookup for EmptyLookup {
    fn effective_model_info(
        &self,
        _model: &str,
        _custom_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        None
    }

    fn raw_cost_map_entry(&self, _key: &str) -> Option<PriceRecord> {
        None
    }

    fn effective_priced_model(
        &self,
        _model: &str,
        _custom_provider: Option<&str>,
    ) -> Option<PricedModel> {
        None
    }
}

fn failure<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "RunwayImagesError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn task_error_name(error: &RunwayTaskError) -> &'static str {
    match error {
        RunwayTaskError::TimedOut { .. } => "timeout",
        RunwayTaskError::Failed { .. } => "failed",
        RunwayTaskError::Cancelled { .. } => "cancelled",
        RunwayTaskError::UnknownStatus { .. } => "unknown_status",
        RunwayTaskError::HttpStatus { .. } => "http_status",
        RunwayTaskError::Decode { .. } | RunwayTaskError::TrailingBytes(_) => "decode",
        RunwayTaskError::Environment { .. }
        | RunwayTaskError::MissingCredentials
        | RunwayTaskError::InvalidHeader(_)
        | RunwayTaskError::TaskUrl(_)
        | RunwayTaskError::TaskId(_)
        | RunwayTaskError::Transport(_)
        | RunwayTaskError::TooLarge { .. } => "other",
    }
}

fn image_error_name(error: &RunwayImageError) -> &'static str {
    match error {
        RunwayImageError::Submission { .. } => "submission",
        RunwayImageError::MissingTaskId => "missing_task_id",
        RunwayImageError::UnsupportedParameter { .. } => "unsupported",
        RunwayImageError::InvalidSize => "invalid_size",
        RunwayImageError::Task(inner) => task_error_name(inner),
    }
}

fn cost_error_name(error: &ImageCostError) -> &'static str {
    match error {
        ImageCostError::UnknownModel { .. } => "unknown_model",
        ImageCostError::NotImageResponse => "not_image_response",
        ImageCostError::WrongFieldType { .. } => "wrong_field_type",
        ImageCostError::Overflow => "overflow",
        _ => "other",
    }
}

fn mapping(input: &Mapping) -> HelperResult {
    let mut results = Vec::with_capacity(input.calls.len());
    for call in &input.calls {
        let attempt = map_openai_params(
            &call.non_default_params,
            call.optional_params.clone(),
            &input.model,
            call.drop_params,
        );
        let Ok(mapped) = attempt else {
            results.push(json!({"unsupported": true}));
            continue;
        };
        let body = request_body(&input.model, &input.prompt, &mapped);
        results.push(json!({"params": mapped, "body": body}));
    }
    Ok(json!({
        "supported": SUPPORTED_OPENAI_PARAMS,
        "results": results,
    }))
}

fn response(input: &ResponseInput) -> HelperResult {
    let config = RunwayImageConfig::for_model(&input.model, &input.environment).map_err(failure)?;
    let transport = Script::new(&input.replies);
    let clock = Clock::new(input.step_millis);
    let transform = RunwayImageTransform {
        transport: &transport,
        clock: &clock,
        settings: &config.settings,
    };
    let outgoing = ImageHttpResponse {
        status_code: input.submission.status,
        headers: input.submission.headers.clone(),
        body: input.submission.body.as_bytes().to_vec(),
    };
    let existing = ImageResponse {
        created: input.created,
        data: (!input.data.is_empty()).then_some(input.data.clone()),
        usage: None,
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(failure)?;
    let outcome =
        runtime.block_on(transform.transform(&outgoing, &input.request_headers, existing));
    match outcome {
        Ok(finished) => Ok(json!({
            "response": finished,
            "gets": transport.describe(),
            "sleeps": *clock.pauses.lock().unwrap_or_else(PoisonError::into_inner),
        })),
        Err(error) => Ok(json!({"error": image_error_name(&error)})),
    }
}

fn cost(input: &Cost) -> HelperResult {
    let lookup = EmptyLookup;
    let mut values = Vec::with_capacity(input.calls.len());
    for call in &input.calls {
        let count = i64::try_from(call.data.len()).map_err(failure)?;
        let billing = ImageBilling {
            image_count: count,
            size: None,
            quality: None,
            usage: None,
        };
        let image = PricedImage {
            model: &input.model,
            response: ImageResponseView::Image(&billing),
            prices: Some(&call.prices),
        };
        let value = match cost_calculator(&image, &lookup) {
            Ok(cost) => json!({"cost": cost.normalize().to_string()}),
            Err(error) => json!({"error": cost_error_name(&error)}),
        };
        values.push(value);
    }
    Ok(json!({"costs": values}))
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Mapping(params) => mapping(params),
            Input::Response(params) => response(params),
            Input::Cost(params) => cost(params),
        }
    }
}
