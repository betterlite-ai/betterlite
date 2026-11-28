#![cfg(test)]

use std::collections::BTreeMap;
use std::str;
use std::time::Duration;

use betterlite_core::api::vector_store_http_write::service::{ClientKind, SuppliedClient};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageParams;
use betterlite_redact::secret::Redactor;
use rstest::rstest;
use tokio::runtime::{Builder, Runtime};

use super::dispatch::{canvas_edit_dispatch, stability_edit_dispatch, unknown_edit_model};
use super::fixture::{
    Costs, DiscardingSink, Factory, FixtureClient, NOW_SECONDS, Prices, Reply, Source,
    logging_state, now, prepost_env,
};
use super::http_error::edit_http_error;
use super::result::edit_result_projection;
use super::signing::edit_signed_request;
use super::transport::edit_transport_selection;
use crate::canvas_results::COST_HEADER;
use crate::image_call::plan::LogContext;
use crate::image_call::transport::ImageRequest;
use crate::image_edits::error::ImageEditError;
use crate::image_edits::execute::dispatch_request;
use crate::image_edits::plan::{EditCall, EditEnvironment, EditImage, EditPlan};
use crate::image_edits::prepare::{prepare_request, resolve_target};
use crate::image_edits::response::{EditReply, ObservedReply, transform_response};

pub(super) const CANVAS_MODEL: &str = "amazon.nova-canvas-v1:0";
pub(super) const STABILITY_MODEL: &str = "stability.stable-image-inpaint-v1:0";
pub(super) const STYLE_MODEL: &str = "stability.stable-image-style-transfer-v1:0";
pub(super) const CANVAS_COST: &str = r#"{"amazon.nova-canvas-v1:0": {"supports_nova_canvas_image_edit": true, "output_cost_per_image": 0.25}}"#;
pub(super) const STABILITY_COST: &str =
    r#"{"stability.stable-image-inpaint-v1:0": {"output_cost_per_image": 0.07}}"#;

pub(super) fn params(pairs: Vec<(&str, Json)>) -> ImageParams {
    pairs
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

pub(super) fn auth_params() -> ImageParams {
    params(vec![
        (
            "aws_access_key_id",
            Json::Text("AKIAIOSFODNN7EXAMPLE".to_owned()),
        ),
        (
            "aws_secret_access_key",
            Json::Text("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_owned()),
        ),
        ("aws_region_name", Json::Text("us-east-1".to_owned())),
    ])
}

pub(super) fn call<'text>(
    model: &'text str,
    prompt: Option<&'text str>,
    optional_params: ImageParams,
) -> EditCall<'text> {
    EditCall {
        model,
        prompt,
        images: &[],
        optional_params,
        api_base: None,
        extra_headers: None,
        api_key: None,
        timeout: None,
        aimage_edit: false,
        environment: EditEnvironment::default(),
        now: now(),
    }
}

pub(super) fn secret(value: &str) -> secrecy::SecretString {
    secrecy::SecretString::from(value.to_owned())
}

pub(super) fn image_list(values: &[&str]) -> Vec<EditImage> {
    values
        .iter()
        .map(|value| EditImage::Text((*value).to_owned()))
        .collect()
}

pub(super) struct Answer {
    pub(super) reply: Reply,
    pub(super) supplied: Option<ClientKind>,
}

pub(super) fn answered(reply: Reply) -> Answer {
    Answer {
        reply,
        supplied: Some(ClientKind::Sync),
    }
}

pub(super) struct Run {
    pub(super) result: Result<EditReply, ImageEditError>,
    pub(super) supplied_requests: Vec<ImageRequest>,
    pub(super) factory_requests: Vec<ImageRequest>,
    pub(super) sync_calls: u32,
    pub(super) async_calls: Vec<Option<Duration>>,
    pub(super) state: LoggingState,
}

impl Run {
    pub(super) fn posted(&self) -> &ImageRequest {
        let requests = if self.supplied_requests.is_empty() {
            &self.factory_requests
        } else {
            &self.supplied_requests
        };
        assert_eq!(requests.len(), 1, "one request reaches the provider");
        &requests[0]
    }

    pub(super) fn headers(&self) -> BTreeMap<String, String> {
        self.posted().headers.iter().cloned().collect()
    }

    pub(super) fn body(&self) -> ImageParams {
        serde_json::from_slice(&self.posted().body).unwrap()
    }

    pub(super) fn text(&self) -> &str {
        str::from_utf8(&self.posted().body).unwrap()
    }

    pub(super) fn response(&self) -> &EditReply {
        self.result.as_ref().unwrap()
    }

    pub(super) fn error(&self) -> &ImageEditError {
        self.result.as_ref().unwrap_err()
    }

    pub(super) fn images(&self) -> Vec<&str> {
        self.response()
            .response
            .data
            .iter()
            .flatten()
            .filter_map(|image| image.b64_json.as_deref())
            .collect()
    }

    // the cost header is the decimal text LiteLLM writes, so the tests compare that text exactly
    pub(super) fn cost(&self) -> Option<String> {
        match self.response().additional_headers.get(COST_HEADER) {
            None => None,
            Some(Json::Number(number)) => Some(number.to_string()),
            Some(other) => panic!("the cost header is a number, got {other:?}"),
        }
    }

    pub(super) fn post_call(&self) -> Option<&Json> {
        self.state.model_call_details.get("original_response")
    }
}

struct Stages<'stage> {
    runtime: &'stage Runtime,
    source: &'stage Source,
    factory: &'stage Factory,
}

pub(super) fn run(call: EditCall<'_>, prices: &str, answer: Answer, redactor: &Redactor) -> Run {
    run_with_source(call, prices, answer, &Source::new(), redactor)
}

pub(super) fn run_with_source(
    mut call: EditCall<'_>,
    prices: &str,
    answer: Answer,
    source: &Source,
    redactor: &Redactor,
) -> Run {
    let costs = Costs::new(prices);
    let context = costs.catalog.context();
    let plan = EditPlan {
        model_cost: &costs.model_cost,
        resolution: &context,
    };
    let client = FixtureClient::new(answer.reply.clone());
    let factory = Factory::new(answer.reply);
    let mut state = logging_state(redactor);
    let sink = DiscardingSink;
    let env = prepost_env(&state, redactor, &sink);
    let runtime = Builder::new_current_thread().build().unwrap();
    let stages = Stages {
        runtime: &runtime,
        source,
        factory: &factory,
    };
    let supplied = answer.supplied.map(|kind| SuppliedClient {
        kind,
        transport: &client,
    });
    let result = {
        let mut log = LogContext {
            state: &mut state,
            env: &env,
        };
        drive(&mut call, &plan, &stages, &mut log, supplied)
    };
    Run {
        result,
        supplied_requests: client.requests(),
        factory_requests: factory.seen_requests(),
        sync_calls: *factory.sync_calls.lock().unwrap(),
        async_calls: factory.async_calls.lock().unwrap().clone(),
        state,
    }
}

fn drive(
    call: &mut EditCall<'_>,
    plan: &EditPlan<'_, '_>,
    stages: &Stages<'_>,
    log: &mut LogContext<'_, '_>,
    supplied: Option<SuppliedClient<'_, FixtureClient>>,
) -> Result<EditReply, ImageEditError> {
    let target = stages
        .runtime
        .block_on(resolve_target(call, stages.source))?;
    let prepared = prepare_request(call, plan, &target, log)?;
    let reply =
        stages
            .runtime
            .block_on(dispatch_request(call, &prepared, supplied, stages.factory))?;
    let observed = ObservedReply {
        prepared: &prepared,
        reply: &reply,
        now_seconds: NOW_SECONDS,
    };
    transform_response(call, plan, &observed, &Prices(plan.model_cost), log)
}

#[rstest]
#[case::canvas_edit_dispatch("canvas_edit_dispatch")]
#[case::stability_edit_dispatch("stability_edit_dispatch")]
#[case::edit_signed_request("edit_signed_request")]
#[case::unknown_edit_model("unknown_edit_model")]
#[case::edit_result_projection("edit_result_projection")]
#[case::edit_http_error("edit_http_error")]
#[case::edit_transport_selection("edit_transport_selection")]
fn image_edits_cases(#[case] name: &str) {
    let redactor = &super::fixture::redactor();
    match name {
        "canvas_edit_dispatch" => canvas_edit_dispatch(redactor),
        "stability_edit_dispatch" => stability_edit_dispatch(redactor),
        "edit_signed_request" => edit_signed_request(redactor),
        "unknown_edit_model" => unknown_edit_model(redactor),
        "edit_result_projection" => edit_result_projection(redactor),
        "edit_http_error" => edit_http_error(redactor),
        "edit_transport_selection" => edit_transport_selection(redactor),
        _ => panic!("unknown case {name}"),
    }
}
