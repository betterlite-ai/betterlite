mod fakes;
mod script;

use std::collections::BTreeMap;
use std::fmt::Display;

use betterlite_core::api::runway::images::cost::cost_calculator;
use betterlite_core::api::runway::images::params::{map_openai_params, request_body};
use betterlite_core::api::runway::images::transform::RunwayImageTransform;
use betterlite_core::api::runway::images::{RunwayImageConfig, SUPPORTED_OPENAI_PARAMS};
use betterlite_core::cost::image_dispatch::policy::{ImageResponseView, PricedImage};
use betterlite_models::image_cost::ImageBilling;
use betterlite_models::price_lookup::PriceRecord;
use betterlite_openai::api::image_contracts::records::{
    ImageData, ImageHeaders, ImageHttpResponse, ImageParams, ImageResponse,
};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::json;

use self::fakes::{Clock, EmptyLookup};
use self::script::Script;

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
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cost {
    model: String,
    prices: PriceRecord,
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
    let finished = runtime
        .block_on(transform.transform(&outgoing, &input.request_headers, existing))
        .map_err(failure)?;
    Ok(json!({
        "response": finished,
        "gets": transport.describe(),
        "sleeps": clock.pauses(),
    }))
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
            prices: Some(&input.prices),
        };
        let price = cost_calculator(&image, &lookup).map_err(failure)?;
        values.push(json!({"cost": price.normalize().to_string()}));
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
