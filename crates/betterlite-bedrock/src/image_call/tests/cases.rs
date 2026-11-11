#![cfg(test)]

use std::collections::BTreeMap;
use std::str;
use std::time::Duration;

use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use rstest::rstest;
use tokio::runtime::{Builder, Runtime};

use super::fixture::{
    DiscardingSink, Factory, FixtureClient, Reply, Source, capture, logging_state, now,
    prepost_env, redactor,
};
use crate::image_call::error::ImageCallError;
use crate::image_call::execute::{dispatch_request, transform_response};
use crate::image_call::plan::{
    ImageCall, ImageEnvironment, LogContext, prepare_request, resolve_target,
};
use crate::image_call::transport::{ClientKind, ImageRequest, SuppliedClient};
use crate::image_models::params::{Field, Params};
use crate::image_models::response::ModelResponse;

pub(super) fn params(pairs: Vec<(&str, Field)>) -> Params {
    pairs
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

pub(super) fn auth_params() -> Params {
    params(vec![
        ("aws_access_key_id", Field::from("AKIAIOSFODNN7EXAMPLE")),
        (
            "aws_secret_access_key",
            Field::from("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"),
        ),
        ("aws_region_name", Field::from("us-east-1")),
    ])
}

pub(super) fn call<'text>(
    model: &'text str,
    prompt: &'text str,
    optional_params: Params,
) -> ImageCall<'text> {
    ImageCall {
        model,
        prompt,
        optional_params,
        api_base: None,
        extra_headers: None,
        api_key: None,
        timeout: None,
        aimg_generation: false,
        environment: ImageEnvironment::default(),
        now: now(),
    }
}

pub(super) fn secret(value: &str) -> secrecy::SecretString {
    secrecy::SecretString::from(value.to_owned())
}

fn field<'params>(params: &'params Params, key: &str) -> &'params Field {
    params.get(key).unwrap()
}

pub(super) struct Run {
    pub(super) result: Result<ModelResponse, ImageCallError>,
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

    pub(super) fn body(&self) -> Params {
        serde_json::from_slice(&self.posted().body).unwrap()
    }

    pub(super) fn text(&self) -> &str {
        str::from_utf8(&self.posted().body).unwrap()
    }

    pub(super) fn response(&self) -> ModelResponse {
        self.result.as_ref().unwrap().clone()
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

pub(super) fn run(call: ImageCall<'_>, reply: Reply, supplied: Option<ClientKind>) -> Run {
    run_with_source(call, reply, supplied, &Source::new())
}

pub(super) fn run_with_source(
    mut call: ImageCall<'_>,
    reply: Reply,
    supplied: Option<ClientKind>,
    source: &Source,
) -> Run {
    let client = FixtureClient::new(reply.clone());
    let factory = Factory::new(reply);
    let mut state = logging_state();
    let redactor = redactor();
    let sink = DiscardingSink;
    let env = prepost_env(&state, &redactor, &sink);
    let runtime = Builder::new_current_thread().build().unwrap();
    let stages = Stages {
        runtime: &runtime,
        source,
        factory: &factory,
    };
    let supplied_ref = supplied.map(|kind| SuppliedClient {
        kind,
        transport: &client,
    });
    let result = {
        let mut log = LogContext {
            state: &mut state,
            env: &env,
        };
        drive(&mut call, &stages, &mut log, supplied_ref)
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
    call: &mut ImageCall<'_>,
    stages: &Stages<'_>,
    log: &mut LogContext<'_, '_>,
    supplied: Option<SuppliedClient<'_, FixtureClient>>,
) -> Result<ModelResponse, ImageCallError> {
    let target = stages
        .runtime
        .block_on(resolve_target(call, stages.source))?;
    let prepared = prepare_request(call, &target, log)?;
    let answered =
        stages
            .runtime
            .block_on(dispatch_request(call, &prepared, supplied, stages.factory))?;
    transform_response(
        call,
        &prepared,
        &answered,
        ModelResponse {
            created: Some(42),
            data: Vec::new(),
        },
        log,
    )
}

#[rstest]
#[case::image_family_dispatch("image_family_dispatch")]
#[case::image_runtime_url("image_runtime_url")]
#[case::signed_json_body("signed_json_body")]
#[case::extra_headers_outside_body("extra_headers_outside_body")]
#[case::base64_image_result("base64_image_result")]
#[case::image_http_error("image_http_error")]
#[case::image_transport_selection("image_transport_selection")]
fn image_call_cases(#[case] name: &str) {
    match name {
        "image_family_dispatch" => {
            family_request_bodies();
            family_reply_records();
            family_precedence();
        }
        "image_runtime_url" => {
            runtime_default_and_configured();
            runtime_api_base_and_environment();
            image_prepare_before_transport();
        }
        "signed_json_body" => {
            signed_body_and_log();
            pre_call_log();
            bearer_authorization();
            signed_authorization_precedence();
        }
        "extra_headers_outside_body" => {
            guardrail_titan_headers();
            guardrail_family_bodies();
            guardrail_null_and_empty();
        }
        "base64_image_result" => {
            titan_base64_and_log();
            ordered_base64_results();
        }
        "image_http_error" => {
            provider_status_error();
            timeout_and_transport_errors();
            reply_decode_errors();
            stability3_finish_reason_error();
        }
        "image_transport_selection" => {
            sync_transport_selection();
            async_transport_selection();
        }
        _ => panic!("unknown case {name}"),
    }
}

fn family_request_bodies() {
    let titan = run(
        call("amazon.titan-image-generator-v2:0", "a cat", auth_params()),
        Reply::ok(r#"{"images": ["dGl0YW4="]}"#),
        Some(ClientKind::Sync),
    );
    assert_eq!(titan.response().data[0].b64_json, "dGl0YW4=");
    assert!(titan.body().contains_key("textToImageParams"));

    let nova = run(
        call("amazon.nova-canvas-v1:0", "a cat", auth_params()),
        Reply::ok(r#"{"images": ["bm92YQ=="]}"#),
        Some(ClientKind::Sync),
    );
    assert_eq!(nova.response().data[0].b64_json, "bm92YQ==");
    assert_eq!(field(&nova.body(), "taskType"), &Field::from("TEXT_IMAGE"));
}

fn family_reply_records() {
    let stability3 = run(
        call("stability.sd3-large-v1:0", "a cat", auth_params()),
        Reply::ok(r#"{"images": ["czM="]}"#),
        Some(ClientKind::Sync),
    );
    assert_eq!(stability3.response().data[0].b64_json, "czM=");
    assert_eq!(
        stability3.body(),
        params(vec![("prompt", Field::from("a cat"))])
    );

    let stability1 = run(
        call("stability.stable-diffusion-xl-v1", "a cat", auth_params()),
        Reply::ok(r#"{"artifacts": [{"base64": "czE="}]}"#),
        Some(ClientKind::Sync),
    );
    assert_eq!(stability1.response().data[0].b64_json, "czE=");
    assert_eq!(
        field(&stability1.body(), "text_prompts"),
        &Field::List(vec![Field::from(params(vec![
            ("text", Field::from("a cat")),
            ("weight", Field::Int(1)),
        ]))])
    );
}

fn family_precedence() {
    // Titan wins when a model name carries both markers, and the original model picks the reply record
    let both = run(
        call(
            "amazon.nova-canvas-amazon.titan-image-generator-v2:0",
            "a cat",
            auth_params(),
        ),
        Reply::ok(r#"{"images": ["Ym90aA=="]}"#),
        Some(ClientKind::Sync),
    );
    assert_eq!(both.response().data[0].b64_json, "Ym90aA==");
    assert!(both.body().contains_key("textToImageParams"));

    // an optional model_id moves the deployment, not the request or response transformation
    let mut overridden = auth_params();
    overridden.insert(
        "model_id".to_owned(),
        Field::from("amazon.nova-canvas-v1:0"),
    );
    let moved = run(
        call("amazon.titan-image-generator-v2:0", "a cat", overridden),
        Reply::ok(r#"{"images": ["dGl0YW4="]}"#),
        Some(ClientKind::Sync),
    );
    assert!(
        moved
            .posted()
            .url
            .ends_with("/model/amazon.nova-canvas-v1%3A0/invoke")
    );
    assert_eq!(moved.response().data[0].b64_json, "dGl0YW4=");
    assert!(moved.body().contains_key("textToImageParams"));
}

fn runtime_default_and_configured() {
    let default = run(
        call("amazon.titan-image-generator-v2:0", "a cat", auth_params()),
        Reply::ok(r#"{"images": []}"#),
        Some(ClientKind::Sync),
    );
    assert_eq!(
        default.posted().url,
        "https://bedrock-runtime.us-east-1.amazonaws.com/model/amazon.titan-image-generator-v2:0/invoke"
    );

    let mut configured_params = auth_params();
    configured_params.insert(
        "aws_bedrock_runtime_endpoint".to_owned(),
        Field::from("https://runtime.example/"),
    );
    let mut configured = call(
        "amazon.titan-image-generator-v2:0",
        "a cat",
        configured_params,
    );
    configured.api_base = Some("https://api.example");
    let proxied = run(
        configured,
        Reply::ok(r#"{"images": []}"#),
        Some(ClientKind::Sync),
    );
    assert_eq!(
        proxied.posted().url,
        "https://runtime.example//model/amazon.titan-image-generator-v2:0/invoke"
    );
}

fn runtime_api_base_and_environment() {
    let mut api_only = call("amazon.titan-image-generator-v2:0", "a cat", auth_params());
    api_only.api_base = Some("https://api.example");
    let direct = run(
        api_only,
        Reply::ok(r#"{"images": []}"#),
        Some(ClientKind::Sync),
    );
    assert_eq!(
        direct.posted().url,
        "https://api.example/model/amazon.titan-image-generator-v2:0/invoke"
    );

    let mut environment = call("amazon.titan-image-generator-v2:0", "a cat", auth_params());
    environment.api_base = Some("https://api.example");
    environment.environment.runtime_endpoint = Some("https://env.example/");
    let from_env = run(
        environment,
        Reply::ok(r#"{"images": []}"#),
        Some(ClientKind::Sync),
    );
    assert_eq!(
        from_env.posted().url,
        "https://env.example//model/amazon.titan-image-generator-v2:0/invoke"
    );
}

fn image_prepare_before_transport() {
    // preparation runs first, so a request that cannot be prepared never creates a client
    let mut broken = auth_params();
    broken.insert("prompt".to_owned(), Field::from("supplied"));
    let failed = run(
        call("stability.sd3-large-v1:0", "a cat", broken),
        Reply::ok(r#"{"images": []}"#),
        None,
    );
    assert!(matches!(failed.result, Err(ImageCallError::Models(_))));
    assert_eq!(failed.sync_calls, 0);
    assert!(failed.factory_requests.is_empty());
}

fn signed_body_and_log() {
    let signed = run(
        call("amazon.titan-image-generator-v2:0", "a cat", auth_params()),
        Reply::ok(r#"{"images": []}"#),
        Some(ClientKind::Sync),
    );
    let headers = signed.headers();
    let authorization = &headers["Authorization"];
    assert!(authorization.starts_with(
        "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20231114/us-east-1/bedrock/aws4_request"
    ));
    assert!(authorization.contains("SignedHeaders=content-type;host;x-amz-date"));
    assert_eq!(headers["X-Amz-Date"], "20231114T221320Z");
    assert_eq!(headers["Content-Type"], "application/json");
    assert_eq!(headers["Content-Length"], "95");
    assert_eq!(
        signed.text(),
        r#"{"taskType": "TEXT_IMAGE", "textToImageParams": {"text": "a cat"}, "imageGenerationConfig": {}}"#
    );
}

fn pre_call_log() {
    let signed = run(
        call("amazon.titan-image-generator-v2:0", "a cat", auth_params()),
        Reply::ok(r#"{"images": []}"#),
        Some(ClientKind::Sync),
    );
    // a failed status skips post_call, so the details still hold what pre_call recorded
    let logged = run(
        call("amazon.titan-image-generator-v2:0", "a cat", auth_params()),
        Reply::status(500, &[], "{}"),
        Some(ClientKind::Sync),
    );
    let details = &logged.state.model_call_details;
    assert_eq!(details.get("input"), Some(&Json::Text("a cat".to_owned())));
    let Some(Json::Object(arguments)) = details.get("additional_args") else {
        panic!("pre_call records its additional arguments");
    };
    assert_eq!(
        arguments.get("api_base"),
        Some(&Json::Text(logged.posted().url.clone()))
    );
    assert!(matches!(
        arguments.get("headers"),
        Some(Json::Object(logged_headers)) if logged_headers.contains_key("Content-Type")
    ));
    assert_eq!(
        arguments
            .get("complete_input_dict")
            .map(|value| serde_json::to_value(value).unwrap()),
        Some(serde_json::to_value(signed.body()).unwrap())
    );
}

fn bearer_authorization() {
    let bearer_token = secret("br-token");
    let mut bearer = call("amazon.titan-image-generator-v2:0", "a cat", auth_params());
    bearer.api_key = Some(&bearer_token);
    let with_bearer = run(
        bearer,
        Reply::ok(r#"{"images": []}"#),
        Some(ClientKind::Sync),
    );
    let bearer_headers = with_bearer.headers();
    assert_eq!(bearer_headers["Authorization"], "Bearer br-token");
    assert!(!bearer_headers.contains_key("X-Amz-Date"));
    assert_eq!(bearer_headers["Content-Length"], "95");

    let configured = secret("env-token");
    let mut environment = call("amazon.titan-image-generator-v2:0", "a cat", auth_params());
    environment.environment.bearer_token = Some(&configured);
    let from_env = run(
        environment,
        Reply::ok(r#"{"images": []}"#),
        Some(ClientKind::Sync),
    );
    assert_eq!(from_env.headers()["Authorization"], "Bearer env-token");
}

fn signed_authorization_precedence() {
    let overridden_headers = vec![("Authorization".to_owned(), "Custom tok".to_owned())];
    let mut overridden = call("amazon.titan-image-generator-v2:0", "a cat", auth_params());
    overridden.extra_headers = Some(&overridden_headers);
    let custom = run(
        overridden,
        Reply::ok(r#"{"images": []}"#),
        Some(ClientKind::Sync),
    );
    let custom_headers = custom.headers();
    assert_eq!(custom_headers["Authorization"], "Custom tok");
    assert_eq!(custom_headers["X-Amz-Date"], "20231114T221320Z");

    let sigv4_headers = vec![(
        "Authorization".to_owned(),
        "AWS4-HMAC-SHA256 caller".to_owned(),
    )];
    let mut sigv4 = call("amazon.titan-image-generator-v2:0", "a cat", auth_params());
    sigv4.extra_headers = Some(&sigv4_headers);
    let kept = run(
        sigv4,
        Reply::ok(r#"{"images": []}"#),
        Some(ClientKind::Sync),
    );
    assert!(
        kept.headers()["Authorization"]
            .starts_with("AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/")
    );
}

fn guardrail_titan_headers() {
    let extra = vec![
        (
            "x-amz-bedrock-guardrail-identifier".to_owned(),
            "extra".to_owned(),
        ),
        ("x-custom".to_owned(), "v".to_owned()),
    ];
    let mut guarded = auth_params();
    guarded.insert("guardrailIdentifier".to_owned(), Field::from("gr"));
    guarded.insert("guardrailVersion".to_owned(), Field::from(""));
    let mut guarded_call = call("amazon.titan-image-generator-v2:0", "a cat", guarded);
    guarded_call.extra_headers = Some(&extra);
    let titan = run(
        guarded_call,
        Reply::ok(r#"{"images": []}"#),
        Some(ClientKind::Sync),
    );
    let headers = titan.headers();
    assert_eq!(headers["x-amz-bedrock-guardrail-identifier"], "gr");
    assert_eq!(headers["x-amz-bedrock-guardrail-version"], "");
    assert_eq!(headers["x-custom"], "v");
    // the optional guardrail replaces the extra header of the same name instead of adding a second one
    assert_eq!(
        titan
            .posted()
            .headers
            .iter()
            .filter(|(name, _)| name == "x-amz-bedrock-guardrail-identifier")
            .count(),
        1
    );
    assert_eq!(titan.body().len(), 3);
    assert_eq!(
        field(&titan.body(), "imageGenerationConfig"),
        &Field::from(Params::new())
    );
}

fn guardrail_family_bodies() {
    let mut nova_params = auth_params();
    nova_params.insert("guardrailIdentifier".to_owned(), Field::from("gr"));
    nova_params.insert("guardrailVersion".to_owned(), Field::from("1"));
    let nova = run(
        call("amazon.nova-canvas-v1:0", "a cat", nova_params),
        Reply::ok(r#"{"images": []}"#),
        Some(ClientKind::Sync),
    );
    assert_eq!(
        field(&nova.body(), "imageGenerationConfig"),
        &Field::from(params(vec![
            ("guardrailIdentifier", Field::from("gr")),
            ("guardrailVersion", Field::from("1")),
        ]))
    );
    assert_eq!(nova.headers()["x-amz-bedrock-guardrail-version"], "1");

    let mut stability1_params = auth_params();
    stability1_params.insert("guardrailIdentifier".to_owned(), Field::from("gr"));
    stability1_params.insert("guardrailVersion".to_owned(), Field::from("1"));
    let stability1 = run(
        call(
            "stability.stable-diffusion-xl-v1",
            "a cat",
            stability1_params,
        ),
        Reply::ok(r#"{"artifacts": []}"#),
        Some(ClientKind::Sync),
    );
    assert_eq!(
        field(&stability1.body(), "guardrailIdentifier"),
        &Field::from("gr")
    );
    assert_eq!(
        field(&stability1.body(), "guardrailVersion"),
        &Field::from("1")
    );
}

fn guardrail_null_and_empty() {
    // a null version is popped without a header, and the already-built Stability 3 body keeps it
    let mut stability3_params = auth_params();
    stability3_params.insert("guardrailIdentifier".to_owned(), Field::from("gr"));
    stability3_params.insert("guardrailVersion".to_owned(), Field::Null);
    let stability3 = run(
        call("stability.sd3-large-v1:0", "a cat", stability3_params),
        Reply::ok(r#"{"images": []}"#),
        Some(ClientKind::Sync),
    );
    assert!(
        !stability3
            .headers()
            .contains_key("x-amz-bedrock-guardrail-version")
    );
    assert_eq!(
        stability3.body().get("guardrailVersion"),
        Some(&Field::Null)
    );

    let mut bad = auth_params();
    bad.insert("guardrailIdentifier".to_owned(), Field::Int(1));
    let refused = run(
        call("amazon.titan-image-generator-v2:0", "a cat", bad),
        Reply::ok(r#"{"images": []}"#),
        Some(ClientKind::Sync),
    );
    assert!(matches!(
        refused.result,
        Err(ImageCallError::GuardrailType("guardrailIdentifier"))
    ));
}

fn titan_base64_and_log() {
    let (captured, _guard) = capture();
    let titan = run(
        call("amazon.titan-image-generator-v2:0", "a cat", auth_params()),
        Reply::ok(r#"{"images": ["b25l", "dHdv"]}"#),
        Some(ClientKind::Sync),
    );
    let response = titan.response();
    assert_eq!(response.created, Some(42));
    assert_eq!(
        response
            .data
            .iter()
            .map(|image| image.b64_json.as_str())
            .collect::<Vec<_>>(),
        ["b25l", "dHdv"]
    );
    let details = &titan.state.model_call_details;
    assert_eq!(
        details.get("original_response"),
        Some(&Json::Text(r#"{"images": ["b25l", "dHdv"]}"#.to_owned()))
    );
    assert_eq!(details.get("input"), Some(&Json::Text("a cat".to_owned())));
    assert!(
        captured
            .text()
            .contains(r#"raw model_response: {"images": ["b25l", "dHdv"]}"#)
    );
}

fn ordered_base64_results() {
    let mut url_params = auth_params();
    url_params.insert("response_format".to_owned(), Field::from("url"));
    let ordered = run(
        call("amazon.nova-canvas-v1:0", "a cat", url_params),
        Reply::ok(r#"{"images": ["dXJs"]}"#),
        Some(ClientKind::Sync),
    );
    assert_eq!(ordered.response().data[0].b64_json, "dXJs");
    assert_eq!(
        field(&ordered.body(), "imageGenerationConfig"),
        &Field::from(params(vec![("response_format", Field::from("url"))]))
    );

    let stability1 = run(
        call("stability.stable-diffusion-xl-v1", "a cat", auth_params()),
        Reply::ok(r#"{"artifacts": [{"base64": "YQ=="}, {"base64": "Yg=="}]}"#),
        Some(ClientKind::Sync),
    );
    assert_eq!(
        stability1
            .response()
            .data
            .iter()
            .map(|image| image.b64_json.as_str())
            .collect::<Vec<_>>(),
        ["YQ==", "Yg=="]
    );
}

fn provider_status_error() {
    let failing = run(
        call("amazon.titan-image-generator-v2:0", "a cat", auth_params()),
        Reply::status(
            429,
            &[("x-amzn-errortype", "ThrottlingException")],
            r#"{"message": "slow down"}"#,
        ),
        Some(ClientKind::Sync),
    );
    match &failing.result {
        Err(ImageCallError::Provider { status, error }) => {
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
    assert!(
        failing.post_call().is_none(),
        "a failed status logs no post_call"
    );
}

fn timeout_and_transport_errors() {
    let timed_out = run(
        call("amazon.titan-image-generator-v2:0", "a cat", auth_params()),
        Reply::Timeout,
        Some(ClientKind::Sync),
    );
    assert!(matches!(
        timed_out.result,
        Err(ImageCallError::Provider { status: 408, .. })
    ));
    assert!(timed_out.post_call().is_none());

    let failed = run(
        call("amazon.titan-image-generator-v2:0", "a cat", auth_params()),
        Reply::Failed,
        Some(ClientKind::Sync),
    );
    assert!(matches!(failed.result, Err(ImageCallError::Transport(_))));
    assert!(failed.post_call().is_none());
}

fn reply_decode_errors() {
    // a reply that parses but does not fit the record still runs post_call first
    let null = run(
        call("amazon.titan-image-generator-v2:0", "a cat", auth_params()),
        Reply::ok("null"),
        Some(ClientKind::Sync),
    );
    assert!(matches!(null.result, Err(ImageCallError::Decode(_))));
    assert!(null.post_call().is_some());

    let malformed = run(
        call("amazon.titan-image-generator-v2:0", "a cat", auth_params()),
        Reply::ok("{not json"),
        Some(ClientKind::Sync),
    );
    assert!(matches!(malformed.result, Err(ImageCallError::Decode(_))));
    assert!(malformed.post_call().is_some());

    let nonobject = run(
        call("amazon.titan-image-generator-v2:0", "a cat", auth_params()),
        Reply::ok("[1, 2]"),
        Some(ClientKind::Sync),
    );
    assert!(matches!(nonobject.result, Err(ImageCallError::Decode(_))));
}

fn stability3_finish_reason_error() {
    let finish_reason = run(
        call("stability.sd3-large-v1:0", "a cat", auth_params()),
        Reply::ok(r#"{"images": [], "finish_reasons": ["ERROR"]}"#),
        Some(ClientKind::Sync),
    );
    assert!(matches!(
        finish_reason.result,
        Err(ImageCallError::Models(_))
    ));
    assert_eq!(
        finish_reason
            .result
            .as_ref()
            .err()
            .and_then(ImageCallError::status),
        Some(400)
    );
    assert!(finish_reason.post_call().is_some());
}

fn sync_transport_selection() {
    let sync_default = run(
        call("amazon.titan-image-generator-v2:0", "a cat", auth_params()),
        Reply::ok(r#"{"images": []}"#),
        None,
    );
    assert_eq!(sync_default.sync_calls, 1);
    assert!(sync_default.async_calls.is_empty());
    assert_eq!(sync_default.factory_requests.len(), 1);

    let sync_with_async = run(
        call("amazon.titan-image-generator-v2:0", "a cat", auth_params()),
        Reply::ok(r#"{"images": []}"#),
        Some(ClientKind::Async),
    );
    assert_eq!(sync_with_async.sync_calls, 1);
    assert!(sync_with_async.supplied_requests.is_empty());

    let sync_supplied = run(
        call("amazon.titan-image-generator-v2:0", "a cat", auth_params()),
        Reply::ok(r#"{"images": []}"#),
        Some(ClientKind::Sync),
    );
    assert_eq!(sync_supplied.sync_calls, 0);
    assert_eq!(sync_supplied.supplied_requests.len(), 1);
}

fn async_transport_selection() {
    let mut async_default_call = call("amazon.titan-image-generator-v2:0", "a cat", auth_params());
    async_default_call.aimg_generation = true;
    let async_default = run(async_default_call, Reply::ok(r#"{"images": []}"#), None);
    assert_eq!(async_default.async_calls, vec![None]);

    let mut timed_call = call("amazon.titan-image-generator-v2:0", "a cat", auth_params());
    timed_call.aimg_generation = true;
    timed_call.timeout = Some(Duration::from_millis(2500));
    let timed = run(timed_call, Reply::ok(r#"{"images": []}"#), None);
    assert_eq!(timed.async_calls, vec![Some(Duration::from_millis(2500))]);

    let mut async_supplied_call = call("amazon.titan-image-generator-v2:0", "a cat", auth_params());
    async_supplied_call.aimg_generation = true;
    let async_supplied = run(
        async_supplied_call,
        Reply::ok(r#"{"images": []}"#),
        Some(ClientKind::Async),
    );
    assert!(async_supplied.async_calls.is_empty());
    assert_eq!(async_supplied.supplied_requests.len(), 1);

    let mut sync_supplied_async_call =
        call("amazon.titan-image-generator-v2:0", "a cat", auth_params());
    sync_supplied_async_call.aimg_generation = true;
    sync_supplied_async_call.timeout = Some(Duration::from_millis(2500));
    let replaced = run(
        sync_supplied_async_call,
        Reply::ok(r#"{"images": []}"#),
        Some(ClientKind::Sync),
    );
    assert_eq!(
        replaced.async_calls,
        vec![Some(Duration::from_millis(2500))]
    );
    assert!(replaced.supplied_requests.is_empty());
}
