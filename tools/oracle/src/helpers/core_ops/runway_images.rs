mod fakes;
mod script;

use std::collections::BTreeMap;
use std::fmt::Display;

use betterlite_core::api::runway::images::cost::cost_calculator;
use betterlite_core::api::runway::images::error::RunwayImageError;
use betterlite_core::api::runway::images::params::{map_openai_params, request_body};
use betterlite_core::api::runway::images::transform::RunwayImageTransform;
use betterlite_core::api::runway::images::{RunwayImageConfig, SUPPORTED_OPENAI_PARAMS};
use betterlite_core::api::runway::tasks::error::RunwayTaskError;
use betterlite_core::cost::image_dispatch::policy::{ImageResponseView, PricedImage};
use betterlite_models::image_cost::ImageBilling;
use betterlite_models::image_cost::error::ImageCostError;
use betterlite_models::price_lookup::PriceRecord;
use betterlite_openai::api::image_contracts::records::{
    ImageData, ImageHeaders, ImageHttpResponse, ImageParams, ImageResponse,
};
use serde::Deserialize;
use serde_json::json;

use self::fakes::{Clock, EmptyLookup};
use self::script::Script;
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

const fn task_error_name(error: &RunwayTaskError) -> &'static str {
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

const fn image_error_name(error: &RunwayImageError) -> &'static str {
    match error {
        RunwayImageError::Submission { .. } => "submission",
        RunwayImageError::MissingTaskId => "missing_task_id",
        RunwayImageError::UnsupportedParameter { .. } => "unsupported",
        RunwayImageError::InvalidSize => "invalid_size",
        RunwayImageError::Task(inner) => task_error_name(inner),
    }
}

const fn cost_error_name(error: &ImageCostError) -> &'static str {
    match error {
        ImageCostError::UnknownModel { .. } => "unknown_model",
        ImageCostError::NotImageResponse => "not_image_response",
        ImageCostError::NoPrice { .. } => "no_price",
        ImageCostError::WrongFieldType { .. } => "wrong_field_type",
        ImageCostError::MalformedSize { .. } => "malformed_size",
        ImageCostError::DimensionOverflow => "dimension_overflow",
        ImageCostError::CounterOverflow => "counter_overflow",
        ImageCostError::Overflow => "overflow",
        ImageCostError::Rate(_) => "rate",
        ImageCostError::Tier(_) => "tier",
        ImageCostError::TokenCost(_) => "token_cost",
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
            "sleeps": clock.pauses(),
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
