use betterlite_models::decisions::Json;

use super::support::{
    FakeCall, Harness, NoInterception, Nothing, OK_RESPONSE, RecordingBridge, Rig, Seams,
    body_json, call, failure, json, kwargs, object, response_json, run_create, text,
};
use crate::api::response_id_codecs::response::build_response_id;
use crate::api::responses_compact_http::service::SharedSession;
use crate::api::responses_create::bridge::Created;
use crate::api::responses_create::error::CreateError;

pub(crate) async fn logged_parameters() {
    let rig = Rig::new();
    rig.wire.queue(200, OK_RESPONSE);
    let gateway = NoInterception;
    let bridge = RecordingBridge::new(&rig.events);
    let nothing = Nothing;
    let seams = Seams::new(&rig, &nothing, &gateway, &bridge);
    let mut first = call("openai/gpt-4o");
    first.api_base = Some(String::from("https://eu.api.openai.com/v1"));
    first.user = Some(String::from("user-1"));
    first.temperature = Some(0.2_f64);
    first.service_tier = Some(String::from("flex"));
    first.kwargs = kwargs(
        r#"{"litellm_call_id":"call-42","model_info":{"id":"deployment_a"},"provider_affinity_header":"X-Session"}"#,
    );
    let mut harness = Harness::new();
    run_create(&seams.deps(), &first, &mut harness)
        .await
        .expect("the logged create answers");
    assert_logged(&harness);
}

fn assert_logged(harness: &Harness) {
    let params = &harness.state.litellm_params;
    assert_eq!(params.get("aresponses"), Some(&Json::Bool(false)));
    assert_eq!(
        params.get("litellm_call_id"),
        Some(&Json::Text(String::from("call-42")))
    );
    assert_eq!(
        params.get("model_info"),
        Some(&json(r#"{"id":"deployment_a"}"#))
    );
    assert_eq!(
        params.get("data_residency"),
        Some(&Json::Text(String::from("eu")))
    );
    assert_eq!(params.get("metadata"), Some(&Json::Null));
    assert_eq!(
        params.get("provider_affinity_header"),
        Some(&Json::Text(String::from("X-Session")))
    );
    assert_eq!(
        params.get("temperature"),
        Some(&json("0.2")),
        "the mapped options are logged"
    );
    assert_eq!(
        params.get("service_tier"),
        Some(&Json::Text(String::from("flex")))
    );
    assert_eq!(
        harness.state.optional_params.get("service_tier"),
        Some(&Json::Text(String::from("flex")))
    );
    assert_eq!(harness.state.model, "gpt-4o");
    assert_eq!(harness.state.user.as_deref(), Some("user-1"));
    assert_eq!(
        harness.state.model_call_details.get("custom_llm_provider"),
        Some(&Json::Text(String::from("openai")))
    );
}

pub(crate) async fn metadata_precedence() {
    let rig = Rig::new();
    rig.wire.queue(200, OK_RESPONSE);
    rig.wire.queue(200, OK_RESPONSE);
    let gateway = NoInterception;
    let bridge = RecordingBridge::new(&rig.events);
    let nothing = Nothing;
    let seams = Seams::new(&rig, &nothing, &gateway, &bridge);
    let mut null_override = call("openai/gpt-4o");
    null_override.metadata = Some(json(r#"{"origin":"named"}"#));
    null_override.kwargs = kwargs(r#"{"litellm_metadata":null}"#);
    let mut harness = Harness::new();
    run_create(&seams.deps(), &null_override, &mut harness)
        .await
        .expect("the null override answers");
    assert_eq!(
        harness.state.litellm_params.get("metadata"),
        Some(&Json::Null),
        "a present null litellm_metadata wins the lookup"
    );
    assert_eq!(
        harness.state.optional_params.get("metadata"),
        Some(&json(r#"{"origin":"named"}"#)),
        "the named metadata stays a provider option"
    );

    let mut fallback = call("openai/gpt-4o");
    fallback.kwargs = kwargs(r#"{"metadata":{"origin":"kwargs"}}"#);
    let mut second = Harness::new();
    run_create(&seams.deps(), &fallback, &mut second)
        .await
        .expect("the kwargs fallback answers");
    assert_eq!(
        second.state.litellm_params.get("metadata"),
        Some(&json(r#"{"origin":"kwargs"}"#))
    );
    assert_eq!(
        second.state.optional_params.get("metadata"),
        Some(&json(r#"{"origin":"kwargs"}"#))
    );
}

pub(crate) async fn residency_and_client() {
    let rig = Rig::new();
    rig.wire.queue(200, OK_RESPONSE);
    rig.wire.queue(200, OK_RESPONSE);
    let gateway = NoInterception;
    let bridge = RecordingBridge::new(&rig.events);
    let nothing = Nothing;
    let seams = Seams::new(&rig, &nothing, &gateway, &bridge);
    let mut first = call("openai/gpt-4o");
    first.api_base = Some(String::from("https://us.api.openai.com/v1"));
    first.ssl_verify = Some(true);
    first.shared_session = Some(SharedSession(7_u64));
    let mut harness = Harness::new();
    run_create(&seams.deps(), &first, &mut harness)
        .await
        .expect("the residency create answers");
    assert_eq!(
        harness.state.litellm_params.get("data_residency"),
        Some(&Json::Text(String::from("us")))
    );
    assert_eq!(rig.source.created(), vec![(Some(true), false)]);
    assert_eq!(
        rig.configs.fake_calls(),
        vec![FakeCall {
            model: String::from("gpt-4o"),
            stream: None,
            provider: String::from("openai"),
        }]
    );

    let mut second = Harness::new();
    run_create(&seams.deps(), &call("openai/gpt-4o"), &mut second)
        .await
        .expect("the plain create answers");
    assert_eq!(
        second.state.litellm_params.get("data_residency"),
        Some(&Json::Null)
    );
    assert_eq!(rig.source.created().len(), 2);
}

pub(crate) async fn stamped_and_synthetic() {
    let rig = Rig::new();
    rig.wire.queue(200, OK_RESPONSE);
    rig.wire.queue(200, OK_RESPONSE);
    let gateway = NoInterception;
    let bridge = RecordingBridge::new(&rig.events);
    let nothing = Nothing;
    let seams = Seams::new(&rig, &nothing, &gateway, &bridge);
    let mut typed = call("openai/gpt-4o");
    typed.kwargs = kwargs(r#"{"litellm_metadata":{"model_info":{"id":"deployment_a"}}}"#);
    let mut harness = Harness::new();
    let created = run_create(&seams.deps(), &typed, &mut harness)
        .await
        .expect("the typed create answers");
    let response = object(&response_json(&created));
    assert_eq!(
        text(&response, "id"),
        build_response_id(Some("openai"), Some("deployment_a"), "resp_a")
    );
    let Created::Responses(stamped) = &created else {
        panic!("the typed answer carries the stamped response");
    };
    assert_eq!(
        stamped.hidden.get("custom_llm_provider"),
        Some(&Json::Text(String::from("openai")))
    );

    synthetic_answer(&seams, &rig, &mut harness).await;
}

async fn synthetic_answer(seams: &Seams<'_>, rig: &Rig, harness: &mut Harness) {
    let mut synthetic = call("openai/fake-stream");
    synthetic.stream = Some(true);
    let synthetic_created = run_create(&seams.deps(), &synthetic, &mut harness)
        .await
        .expect("the fake stream answers");
    let Created::Synthetic(answer) = &synthetic_created else {
        panic!("a fake stream yields the synthetic answer");
    };
    assert_eq!(answer.response.id, "resp_a");
    let fakes = rig.configs.fake_calls();
    assert_eq!(fakes.len(), 2);
    assert_eq!(fakes[1].model, "fake-stream");
    assert_eq!(fakes[1].stream, Some(true));
    assert_eq!(fakes[1].provider, "openai");
    let body = body_json(&rig.wire.requests()[1]);
    assert!(object(&body).get("stream").is_none());
}

pub(crate) async fn provider_failure() {
    let rig = Rig::new();
    rig.wire.queue(
        401,
        r#"{"error":{"message":"invalid api key","type":"invalid_request_error","param":null,"code":"invalid_api_key"}}"#,
    );
    let gateway = NoInterception;
    let bridge = RecordingBridge::new(&rig.events);
    let nothing = Nothing;
    let seams = Seams::new(&rig, &nothing, &gateway, &bridge);
    let mut harness = Harness::new();
    let error = run_create(&seams.deps(), &call("openai/gpt-4o"), &mut harness)
        .await
        .expect_err("the provider failure is reported");
    assert!(matches!(error, CreateError::Request(_)));
    let failure = failure(&error);
    assert_eq!(failure.class, "AuthenticationError");
    assert_eq!(failure.status, 401);
    assert_eq!(failure.kind, None);
    assert_eq!(failure.param, None);
    assert_eq!(failure.code, None);
    assert!(failure.message.contains("AuthenticationError"));
}
