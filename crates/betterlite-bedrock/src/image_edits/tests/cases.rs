#![cfg(test)]

use std::collections::BTreeMap;
use std::str;
use std::time::Duration;

use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{ImageParams, ImageUsage};
use betterlite_redact::secret::Redactor;
use rstest::rstest;
use tokio::runtime::{Builder, Runtime};

use super::fixture::{
    Costs, DiscardingSink, Factory, FixtureClient, NOW_SECONDS, Prices, Reply, Source, capture,
    logging_state, now, prepost_env,
};
use crate::canvas_edits::error::CanvasEditError;
use crate::canvas_results::COST_HEADER;
use crate::image_call::plan::LogContext;
use crate::image_call::transport::{ClientKind, ImageRequest, SuppliedClient};
use crate::image_edits::error::ImageEditError;
use crate::image_edits::execute::dispatch_request;
use crate::image_edits::plan::{EditCall, EditEnvironment, EditImage, EditPlan, PreparedRequest};
use crate::image_edits::prepare::{prepare_request, resolve_target};
use crate::image_edits::response::{EditReply, ObservedReply, transform_response};
use crate::signing::request::AwsSignedRequest;
use crate::stability_edits::error::{StabilityEditError, StabilityReplyError};

const CANVAS_MODEL: &str = "amazon.nova-canvas-v1:0";
const STABILITY_MODEL: &str = "stability.stable-image-inpaint-v1:0";
const STYLE_MODEL: &str = "stability.stable-image-style-transfer-v1:0";
const CANVAS_COST: &str = r#"{"amazon.nova-canvas-v1:0": {"supports_nova_canvas_image_edit": true, "output_cost_per_image": 0.25}}"#;
const STABILITY_COST: &str =
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

fn image_list(values: &[&str]) -> Vec<EditImage> {
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

fn canvas_edit_dispatch(redactor: &Redactor) {
    // the raw model_cost flag picks the Nova Canvas config, and its body names the task first
    let supplied = image_list(&["aW1n"]);
    let mut options = auth_params();
    options.insert("numberOfImages".to_owned(), Json::Number(2_i64.into()));
    options.insert("width".to_owned(), Json::Number(512_i64.into()));
    let mut canvas = call(CANVAS_MODEL, Some("edit it"), options);
    canvas.images = &supplied;
    let variation = run(
        canvas,
        CANVAS_COST,
        answered(Reply::ok(r#"{"images": ["b3V0"]}"#)),
        redactor,
    );
    assert_eq!(variation.images(), ["b3V0"]);
    assert_eq!(variation.cost().as_deref(), Some("0.25"));
    assert_eq!(
        variation.text(),
        r#"{"taskType": "IMAGE_VARIATION", "imageVariationParams": {"images": ["aW1n"], "text": "edit it"}, "imageGenerationConfig": {"width": 512, "numberOfImages": 2}}"#
    );

    // a second image never reaches the transform: LiteLLM hands it image[0] alone
    let extra = image_list(&["aW1n", "dHdv"]);
    let mut many = call(CANVAS_MODEL, Some("edit it"), auth_params());
    many.images = &extra;
    let first_only = run(
        many,
        CANVAS_COST,
        answered(Reply::ok(r#"{"images": ["b3V0"]}"#)),
        redactor,
    );
    assert_eq!(
        first_only.text(),
        r#"{"taskType": "IMAGE_VARIATION", "imageVariationParams": {"images": ["aW1n"], "text": "edit it"}}"#
    );

    // BACKGROUND_REMOVAL keeps only its own params record, even with a generation option set
    let mut removal_options = auth_params();
    removal_options.insert(
        "taskType".to_owned(),
        Json::Text("BACKGROUND_REMOVAL".to_owned()),
    );
    removal_options.insert("numberOfImages".to_owned(), Json::Number(2_i64.into()));
    let mut removal = call(CANVAS_MODEL, None, removal_options);
    removal.images = &supplied;
    let removed = run(
        removal,
        CANVAS_COST,
        answered(Reply::ok(r#"{"images": ["YmFjaw=="]}"#)),
        redactor,
    );
    assert_eq!(
        removed.text(),
        r#"{"taskType": "BACKGROUND_REMOVAL", "backgroundRemovalParams": {"image": "aW1n"}}"#
    );
}

fn stability_edit_dispatch(redactor: &Redactor) {
    // the name markers pick Stability, and the body fixes the png output format before the prompt
    let supplied = image_list(&["aW1n"]);
    let mut options = auth_params();
    options.insert("aspect_ratio".to_owned(), Json::Text("16:9".to_owned()));
    options.insert("seed".to_owned(), Json::Number(7_i64.into()));
    options.insert("quality".to_owned(), Json::Text("standard".to_owned()));
    let mut stability = call(STABILITY_MODEL, Some("edit it"), options);
    stability.images = &supplied;
    let edited = run(
        stability,
        STABILITY_COST,
        answered(Reply::ok(r#"{"images": ["c3RhYg=="]}"#)),
        redactor,
    );
    assert_eq!(edited.images(), ["c3RhYg=="]);
    assert_eq!(edited.cost().as_deref(), Some("0.07"));
    assert_eq!(
        edited.text(),
        r#"{"output_format": "png", "prompt": "edit it", "image": "aW1n", "aspect_ratio": "16:9", "seed": 7}"#
    );

    // a style-transfer model names its source image init_image
    let mut transfer = call(STYLE_MODEL, Some("edit it"), auth_params());
    transfer.images = &supplied;
    let styled = run(
        transfer,
        "{}",
        answered(Reply::ok(r#"{"images": ["c3R5bGU="]}"#)),
        redactor,
    );
    assert_eq!(
        styled.text(),
        r#"{"output_format": "png", "prompt": "edit it", "init_image": "aW1n"}"#
    );
    // an unmapped stability model still answers, at the zero price
    assert_eq!(styled.cost().as_deref(), Some("0.0"));

    // without a prompt the key is absent, and pre_call records a null input
    let mut bare = call(STABILITY_MODEL, None, auth_params());
    bare.images = &supplied;
    let promptless = run(
        bare,
        STABILITY_COST,
        answered(Reply::ok(r#"{"images": ["cXVpZXQ="]}"#)),
        redactor,
    );
    assert_eq!(
        promptless.text(),
        r#"{"output_format": "png", "image": "aW1n"}"#
    );
    assert_eq!(
        promptless.state.model_call_details.get("input"),
        Some(&Json::Null)
    );

    // Stability permits no image at all, and an empty image list is a successful result
    let no_images: &[EditImage] = &[];
    let mut imageless = call(STABILITY_MODEL, Some("edit it"), auth_params());
    imageless.images = no_images;
    let empty = run(
        imageless,
        STABILITY_COST,
        answered(Reply::ok(r#"{"images": []}"#)),
        redactor,
    );
    assert_eq!(
        empty.text(),
        r#"{"output_format": "png", "prompt": "edit it"}"#
    );
    assert!(empty.images().is_empty());
    assert_eq!(empty.cost().as_deref(), Some("0.07"));
}

fn edit_signed_request(redactor: &Redactor) {
    let supplied = image_list(&["aW1n"]);
    let mut signed_call = call(STABILITY_MODEL, Some("edit it"), auth_params());
    signed_call.images = &supplied;
    let signed = run(
        signed_call,
        "{}",
        answered(Reply::ok(r#"{"images": ["aQ=="]}"#)),
        redactor,
    );
    assert_eq!(
        signed.posted().url,
        "https://bedrock-runtime.us-east-1.amazonaws.com/model/stability.stable-image-inpaint-v1:0/invoke"
    );
    assert_eq!(
        signed.text(),
        r#"{"output_format": "png", "prompt": "edit it", "image": "aW1n"}"#
    );
    let headers = signed.headers();
    assert_eq!(headers["Content-Type"], "application/json");
    assert_eq!(headers["Content-Length"], signed.text().len().to_string());
    assert!(headers["Authorization"].starts_with(
        "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20231114/us-east-1/bedrock/aws4_request"
    ));
    assert!(headers["Authorization"].contains("SignedHeaders=content-type;host;x-amz-date"));
    assert_eq!(headers["X-Amz-Date"], "20231114T221320Z");

    // pre_call records the signed headers, the endpoint and the prepared body
    let arguments = match signed.state.model_call_details.get("additional_args") {
        Some(Json::Object(arguments)) => arguments,
        other => panic!("pre_call records its arguments, got {other:?}"),
    };
    assert_eq!(
        arguments.get("api_base"),
        Some(&Json::Text(signed.posted().url.clone()))
    );
    assert!(matches!(
        arguments.get("headers"),
        Some(Json::Object(logged)) if logged.contains_key("Authorization")
    ));
    assert_eq!(
        arguments
            .get("complete_input_dict")
            .map(|value| serde_json::to_value(value).unwrap()),
        Some(serde_json::to_value(signed.body()).unwrap())
    );

    // an extra header of an exact name replaces the built-in one; a new one rides along
    let extra = [
        ("X-Custom".to_owned(), "v".to_owned()),
        (
            "Content-Type".to_owned(),
            "application/json; charset=utf-8".to_owned(),
        ),
    ];
    let mut customized_call = call(STABILITY_MODEL, Some("edit it"), auth_params());
    customized_call.images = &supplied;
    customized_call.extra_headers = Some(&extra);
    let customized = run(
        customized_call,
        "{}",
        answered(Reply::ok(r#"{"images": ["aQ=="]}"#)),
        redactor,
    );
    let custom_headers = customized.headers();
    assert_eq!(
        custom_headers["Content-Type"],
        "application/json; charset=utf-8"
    );
    assert_eq!(custom_headers["X-Custom"], "v");
    assert!(!customized.body().contains_key("X-Custom"));

    // an api_key signs as a bearer token and drops the SigV4 date
    let token = secret("br-token");
    let mut bearer_call = call(STABILITY_MODEL, Some("edit it"), auth_params());
    bearer_call.images = &supplied;
    bearer_call.api_key = Some(&token);
    let bearer = run(
        bearer_call,
        "{}",
        answered(Reply::ok(r#"{"images": ["aQ=="]}"#)),
        redactor,
    );
    assert_eq!(bearer.headers()["Authorization"], "Bearer br-token");
    assert!(!bearer.headers().contains_key("X-Amz-Date"));

    // a configured endpoint wins over the default, a trailing slash doubles as Python's f-string does
    let mut endpoint_options = auth_params();
    endpoint_options.insert(
        "aws_bedrock_runtime_endpoint".to_owned(),
        Json::Text("https://runtime.example/".to_owned()),
    );
    let mut endpoint_call = call(STABILITY_MODEL, Some("edit it"), endpoint_options);
    endpoint_call.images = &supplied;
    let configured = run(
        endpoint_call,
        "{}",
        answered(Reply::ok(r#"{"images": ["aQ=="]}"#)),
        redactor,
    );
    assert_eq!(
        configured.posted().url,
        "https://runtime.example//model/stability.stable-image-inpaint-v1:0/invoke"
    );

    // api_base is the configured endpoint the caller passes, and it wins over the default
    let mut api_call = call(STABILITY_MODEL, Some("edit it"), auth_params());
    api_call.images = &supplied;
    api_call.api_base = Some("https://api.example");
    let proxied = run(
        api_call,
        "{}",
        answered(Reply::ok(r#"{"images": ["aQ=="]}"#)),
        redactor,
    );
    assert_eq!(
        proxied.posted().url,
        "https://api.example/model/stability.stable-image-inpaint-v1:0/invoke"
    );

    // the transport sends the stored endpoint and prepared body, not the signed URL record
    let direct_call = call(STABILITY_MODEL, None, ImageParams::default());
    let prepared = PreparedRequest {
        endpoint_url: "https://stored.example/model/m/invoke".to_owned(),
        signed: AwsSignedRequest {
            method: "POST".to_owned(),
            url: "https://signed.example/elsewhere".to_owned(),
            headers: vec![("X-Prepared".to_owned(), "yes".to_owned())],
            body: b"{\"stored\": true}".to_vec(),
        },
        data: ImageParams::default(),
    };
    let direct_client = FixtureClient::new(Reply::ok(r#"{"images": ["aQ=="]}"#));
    let direct_factory = Factory::new(Reply::ok(r#"{"images": []}"#));
    let runtime = Builder::new_current_thread().build().unwrap();
    let reply = runtime
        .block_on(dispatch_request(
            &direct_call,
            &prepared,
            Some(SuppliedClient {
                kind: ClientKind::Sync,
                transport: &direct_client,
            }),
            &direct_factory,
        ))
        .unwrap();
    assert_eq!(reply.status, 200);
    let sent_requests = direct_client.requests();
    let sent = &sent_requests[0];
    assert_eq!(sent.url, "https://stored.example/model/m/invoke");
    assert_eq!(sent.body, b"{\"stored\": true}".to_vec());
    assert_eq!(
        sent.headers,
        vec![("X-Prepared".to_owned(), "yes".to_owned())]
    );
}

fn unknown_edit_model(redactor: &Redactor) {
    // without a stability marker and without the nova-canvas flag there is no config class to build
    for prices in [
        "{}",
        r#"{"unknown-fixture": {}}"#,
        r#"{"unknown-fixture": {"supports_nova_canvas_image_edit": "true"}}"#,
    ] {
        let unknown = run(
            call("unknown-fixture", None, auth_params()),
            prices,
            Answer {
                reply: Reply::ok("{}"),
                supplied: None,
            },
            redactor,
        );
        assert!(matches!(
            unknown.error(),
            ImageEditError::Canvas(CanvasEditError::UnsupportedModel { .. })
        ));
        assert_eq!(
            unknown.error().to_string(),
            "Unsupported Bedrock image-edit model: 'unknown-fixture'. Use a stability.* image-edit model id or add supports_nova_canvas_image_edit in model_prices for this id."
        );
        assert_eq!(unknown.error().status(), None);
        assert_eq!(unknown.sync_calls, 0);
        assert!(unknown.factory_requests.is_empty());
        assert!(unknown.supplied_requests.is_empty());
    }
}

fn edit_result_projection(redactor: &Redactor) {
    let (captured, _guard) = capture();
    let supplied = image_list(&["aW1n"]);

    // the canvas reply keeps the instant and a default usage record, and prices every returned image
    let mut canvas_call = call(CANVAS_MODEL, Some("edit it"), auth_params());
    canvas_call.images = &supplied;
    let canvas = run(
        canvas_call,
        CANVAS_COST,
        answered(Reply::ok(r#"{"images": ["b25l", "dHdv"]}"#)),
        redactor,
    );
    let reply = canvas.response();
    assert_eq!(reply.response.created, NOW_SECONDS);
    assert_eq!(reply.response.usage, Some(ImageUsage::default()));
    assert_eq!(canvas.images(), ["b25l", "dHdv"]);
    assert_eq!(canvas.cost().as_deref(), Some("0.5"));
    assert_eq!(
        canvas.state.model_call_details.get("original_response"),
        Some(&Json::Text(r#"{"images": ["b25l", "dHdv"]}"#.to_owned()))
    );
    assert!(
        captured
            .text()
            .contains(r#"raw model_response: {"images": ["b25l", "dHdv"]}"#)
    );

    // the stability reply has no usage record and prices the call once
    let mut stability_call = call(STABILITY_MODEL, Some("edit it"), auth_params());
    stability_call.images = &supplied;
    let stability = run(
        stability_call,
        STABILITY_COST,
        answered(Reply::ok(
            r#"{"images": ["c3RhYg=="], "finish_reasons": [null]}"#,
        )),
        redactor,
    );
    assert_eq!(stability.response().response.usage, None);
    assert_eq!(stability.images(), ["c3RhYg=="]);
    assert_eq!(stability.cost().as_deref(), Some("0.07"));

    // a body of null stops at the handler's own check, after post_call already recorded the text
    let mut null_call = call(STABILITY_MODEL, None, auth_params());
    null_call.images = &supplied;
    let null = run(null_call, "{}", answered(Reply::ok("null")), redactor);
    assert!(matches!(null.error(), ImageEditError::NullReply));
    assert_eq!(
        null.error().to_string(),
        "Error in response object format, got None"
    );
    assert!(null.post_call().is_some());

    // text that is not JSON is the outer parse failure, and post_call still ran first
    let mut malformed_call = call(STABILITY_MODEL, None, auth_params());
    malformed_call.images = &supplied;
    let malformed = run(
        malformed_call,
        "{}",
        answered(Reply::ok("{not json")),
        redactor,
    );
    assert!(matches!(malformed.error(), ImageEditError::Json(_)));
    assert!(malformed.post_call().is_some());

    // a parsed list passes the outer check and the family names the shape
    let mut list_call = call(STABILITY_MODEL, None, auth_params());
    list_call.images = &supplied;
    let list = run(list_call, "{}", answered(Reply::ok("[]")), redactor);
    assert!(matches!(
        list.error(),
        ImageEditError::Stability(StabilityEditError::Reply(StabilityReplyError::NotAnObject))
    ));
    assert!(list.post_call().is_some());

    // a canvas reply carrying errors reports through the canvas config at the upstream status
    let mut failed_call = call(CANVAS_MODEL, None, auth_params());
    failed_call.images = &supplied;
    let failed = run(
        failed_call,
        CANVAS_COST,
        answered(Reply::ok(r#"{"errors": ["content filtered"]}"#)),
        redactor,
    );
    assert!(matches!(failed.error(), ImageEditError::CanvasReply(_)));
    assert_eq!(failed.error().status(), Some(200));
}

fn edit_http_error(redactor: &Redactor) {
    let supplied = image_list(&["aW1n"]);

    let mut throttled_call = call(STABILITY_MODEL, Some("edit it"), auth_params());
    throttled_call.images = &supplied;
    let throttled = run(
        throttled_call,
        "{}",
        answered(Reply::status(
            429,
            &[("x-amzn-errortype", "ThrottlingException")],
            r#"{"message": "slow down"}"#,
        )),
        redactor,
    );
    match throttled.error() {
        ImageEditError::Provider { status, error } => {
            assert_eq!(*status, 429);
            assert_eq!(error.error.status_code, 429);
            assert_eq!(error.error.message, r#"{"message": "slow down"}"#);
            assert_eq!(
                error.response_headers,
                vec![(
                    "x-amzn-errortype".to_owned(),
                    "ThrottlingException".to_owned()
                )]
            );
        }
        other => panic!("expected a provider error, got {other:?}"),
    }
    assert_eq!(throttled.error().status(), Some(429));
    assert!(
        throttled.post_call().is_none(),
        "a failed status logs no post_call"
    );

    // httpx treats every non-2xx as an error, so a redirect is a provider error too
    let mut redirect_call = call(STABILITY_MODEL, Some("edit it"), auth_params());
    redirect_call.images = &supplied;
    let redirect = run(
        redirect_call,
        "{}",
        answered(Reply::status(302, &[], r#"{"images": []}"#)),
        redactor,
    );
    assert_eq!(redirect.error().status(), Some(302));
    assert!(redirect.post_call().is_none());

    // a body that is not UTF-8 still reports a provider error, naming its byte count
    let mut binary_call = call(STABILITY_MODEL, Some("edit it"), auth_params());
    binary_call.images = &supplied;
    let binary = run(
        binary_call,
        "{}",
        answered(Reply::bytes(500, &[], vec![0xFF, 0xFE])),
        redactor,
    );
    match binary.error() {
        ImageEditError::Provider { status, error } => {
            assert_eq!(*status, 500);
            assert_eq!(error.error.message, "2 bytes that are not UTF-8");
        }
        other => panic!("expected a provider error, got {other:?}"),
    }

    // a timeout reports 408, and a failed transfer keeps the transport error
    let mut timed_call = call(STABILITY_MODEL, Some("edit it"), auth_params());
    timed_call.images = &supplied;
    let timed = run(timed_call, "{}", answered(Reply::Timeout), redactor);
    match timed.error() {
        ImageEditError::Provider { status, error } => {
            assert_eq!(*status, 408);
            assert_eq!(error.error.message, "the provider request timed out");
        }
        other => panic!("expected a provider error, got {other:?}"),
    }
    assert!(timed.post_call().is_none());

    let mut broken_call = call(STABILITY_MODEL, Some("edit it"), auth_params());
    broken_call.images = &supplied;
    let broken = run(broken_call, "{}", answered(Reply::Failed), redactor);
    assert!(matches!(broken.error(), ImageEditError::Transport(_)));
    assert!(broken.post_call().is_none());
}

fn edit_transport_selection(redactor: &Redactor) {
    let supplied = image_list(&["aW1n"]);

    // sync: the factory builds the default client, a supplied sync client replaces it
    let mut sync_default_call = call(STABILITY_MODEL, Some("edit it"), auth_params());
    sync_default_call.images = &supplied;
    let sync_default = run(
        sync_default_call,
        "{}",
        Answer {
            reply: Reply::ok(r#"{"images": ["aQ=="]}"#),
            supplied: None,
        },
        redactor,
    );
    assert_eq!(sync_default.sync_calls, 1);
    assert!(sync_default.async_calls.is_empty());
    assert_eq!(sync_default.factory_requests.len(), 1);

    let mut async_given_call = call(STABILITY_MODEL, Some("edit it"), auth_params());
    async_given_call.images = &supplied;
    let async_given = run(
        async_given_call,
        "{}",
        Answer {
            reply: Reply::ok(r#"{"images": ["aQ=="]}"#),
            supplied: Some(ClientKind::Async),
        },
        redactor,
    );
    assert_eq!(async_given.sync_calls, 1);
    assert!(async_given.supplied_requests.is_empty());

    let mut sync_given_call = call(STABILITY_MODEL, Some("edit it"), auth_params());
    sync_given_call.images = &supplied;
    let sync_given = run(
        sync_given_call,
        "{}",
        answered(Reply::ok(r#"{"images": ["aQ=="]}"#)),
        redactor,
    );
    assert_eq!(sync_given.sync_calls, 0);
    assert_eq!(sync_given.supplied_requests.len(), 1);

    // async: the default client takes the call timeout, and a supplied async client replaces it
    let mut async_default_call = call(STABILITY_MODEL, Some("edit it"), auth_params());
    async_default_call.images = &supplied;
    async_default_call.aimage_edit = true;
    async_default_call.timeout = Some(Duration::from_millis(2500));
    let async_default = run(
        async_default_call,
        "{}",
        Answer {
            reply: Reply::ok(r#"{"images": ["aQ=="]}"#),
            supplied: None,
        },
        redactor,
    );
    assert_eq!(
        async_default.async_calls,
        vec![Some(Duration::from_millis(2500))]
    );
    assert_eq!(async_default.factory_requests.len(), 1);

    // a caller with no timeout hands the async client a None timeout all the same
    let mut async_none_call = call(STABILITY_MODEL, Some("edit it"), auth_params());
    async_none_call.images = &supplied;
    async_none_call.aimage_edit = true;
    let async_none = run(
        async_none_call,
        "{}",
        Answer {
            reply: Reply::ok(r#"{"images": ["aQ=="]}"#),
            supplied: None,
        },
        redactor,
    );
    assert_eq!(async_none.async_calls, vec![None]);
    assert_eq!(async_none.factory_requests.len(), 1);

    let mut async_supplied_call = call(STABILITY_MODEL, Some("edit it"), auth_params());
    async_supplied_call.images = &supplied;
    async_supplied_call.aimage_edit = true;
    let async_supplied = run(
        async_supplied_call,
        "{}",
        Answer {
            reply: Reply::ok(r#"{"images": ["aQ=="]}"#),
            supplied: Some(ClientKind::Async),
        },
        redactor,
    );
    assert!(async_supplied.async_calls.is_empty());
    assert_eq!(async_supplied.supplied_requests.len(), 1);

    // a sync client on the async path is not accepted, so the factory builds the async one
    let mut sync_on_async_call = call(STABILITY_MODEL, Some("edit it"), auth_params());
    sync_on_async_call.images = &supplied;
    sync_on_async_call.aimage_edit = true;
    sync_on_async_call.timeout = Some(Duration::from_millis(2500));
    let replaced = run(
        sync_on_async_call,
        "{}",
        answered(Reply::ok(r#"{"images": ["aQ=="]}"#)),
        redactor,
    );
    assert_eq!(
        replaced.async_calls,
        vec![Some(Duration::from_millis(2500))]
    );
    assert!(replaced.supplied_requests.is_empty());
}
