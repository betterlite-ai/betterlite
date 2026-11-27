use std::time::Duration;

use betterlite_error::error::ErrorClass;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_models::decisions::Json;

use super::support::{
    DELETED, Operation, REQUEST_TIMEOUT, RealConfigs, RecordingLookup, Rig,
    assert_forwarded_headers, assert_line, assert_no_request, call, deleted, head_value, headers,
    json_server, kwargs, object, openai_kwargs, provider, run, text, url,
};
use crate::api::eval_lifecycle::{DEFAULT_OPENAI_API_BASE, EvalLifecycleCall};

#[tokio::test]
async fn delete_eval_returns_the_provider_delete_response() {
    let serving = json_server("200 OK", DELETED, Duration::ZERO);
    let mut rig = Rig::new(RealConfigs, TransportLimits::DEFAULT, REQUEST_TIMEOUT);
    let params = openai_kwargs(&url(&serving));
    let request = call("eval_1", &params);
    let answer = run(&mut rig, Operation::Delete, &request)
        .await
        .expect("the fixture answers");
    let response = deleted(answer);
    let seen = serving.finish().expect("the fixture finishes");
    assert_line(&seen, "DELETE", "/v1/evals/eval_1");
    assert!(seen.body.is_empty(), "a delete carries no body");
    assert_eq!(
        head_value(&seen.head, "authorization"),
        Some("Bearer sk-test"),
        "the environment is validated even without extra headers"
    );
    assert_eq!(response.eval_id, "eval_1");
    assert_eq!(response.object, "eval.deleted");
    assert!(response.deleted);
}

// B523's validate_environment adds the Authorization header, the transform is handed that answer, and the handler sends what the transform returned
#[tokio::test]
async fn delete_eval_passes_the_validated_headers_to_the_transform() {
    let serving = json_server("200 OK", DELETED, Duration::ZERO);
    let base = url(&serving);
    let mut rig = Rig::new(
        RecordingLookup::new(base.clone()),
        TransportLimits::DEFAULT,
        REQUEST_TIMEOUT,
    );
    let params = openai_kwargs("https://records.example");
    let extra = headers(&[("x-extra", "one")]);
    let request = EvalLifecycleCall {
        extra_headers: Some(&extra),
        ..call("eval_1", &params)
    };
    let _answer = run(&mut rig, Operation::Delete, &request)
        .await
        .expect("the fixture answers");
    let seen = serving.finish().expect("the fixture finishes");
    let asked = rig.configs.recorder().only_call();
    assert_forwarded_headers(&seen, &asked);
}

#[tokio::test]
async fn delete_eval_uses_the_api_base_from_the_keywords() {
    let serving = json_server("200 OK", DELETED, Duration::ZERO);
    let base = url(&serving);
    let mut rig = Rig::new(
        RecordingLookup::new(base.clone()),
        TransportLimits::DEFAULT,
        REQUEST_TIMEOUT,
    );
    let params = openai_kwargs("https://records.example");
    let request = call("eval_1", &params);
    let _answer = run(&mut rig, Operation::Delete, &request)
        .await
        .expect("the fixture answers");
    serving.finish().expect("the fixture finishes");
    assert_eq!(
        rig.configs.recorder().only_call().api_base,
        "https://records.example"
    );
}

#[tokio::test]
async fn delete_eval_defaults_the_api_base_to_openai() {
    let serving = json_server("200 OK", DELETED, Duration::ZERO);
    let base = url(&serving);
    let mut rig = Rig::new(
        RecordingLookup::new(base.clone()),
        TransportLimits::DEFAULT,
        REQUEST_TIMEOUT,
    );
    let params = kwargs(&[("api_key", text("sk-test"))]);
    let request = call("eval_1", &params);
    let _answer = run(&mut rig, Operation::Delete, &request)
        .await
        .expect("the fixture answers");
    serving.finish().expect("the fixture finishes");
    assert_eq!(
        rig.configs.recorder().only_call().api_base,
        DEFAULT_OPENAI_API_BASE
    );
}

// litellm_params.api_base or DEFAULT_OPENAI_API_BASE takes an empty string as absent, as Python's `or` does
#[tokio::test]
async fn delete_eval_treats_an_empty_api_base_as_unset() {
    let serving = json_server("200 OK", DELETED, Duration::ZERO);
    let base = url(&serving);
    let mut rig = Rig::new(
        RecordingLookup::new(base),
        TransportLimits::DEFAULT,
        REQUEST_TIMEOUT,
    );
    let params = kwargs(&[("api_key", text("sk-test")), ("api_base", text(""))]);
    let request = call("eval_1", &params);
    let _answer = run(&mut rig, Operation::Delete, &request)
        .await
        .expect("the fixture answers");
    serving.finish().expect("the fixture finishes");
    assert_eq!(
        rig.configs.recorder().only_call().api_base,
        DEFAULT_OPENAI_API_BASE
    );
}

#[tokio::test]
async fn delete_eval_logs_the_eval_id_before_the_send() {
    let serving = json_server("200 OK", DELETED, Duration::ZERO);
    let target = format!("{}/v1/evals/eval_1", url(&serving));
    let mut rig = Rig::new(RealConfigs, TransportLimits::DEFAULT, REQUEST_TIMEOUT);
    let mut params = openai_kwargs(&url(&serving));
    params.insert("litellm_call_id".to_owned(), text("call-9"));
    let request = call("eval_1", &params);
    let _answer = run(&mut rig, Operation::Delete, &request)
        .await
        .expect("the fixture answers");
    serving.finish().expect("the fixture finishes");
    assert_eq!(
        rig.fixture.state.optional_params.get("eval_id"),
        Some(&text("eval_1"))
    );
    assert_eq!(rig.fixture.detail("eval_id"), Some(&text("eval_1")));
    assert_eq!(
        rig.fixture.state.custom_llm_provider.as_deref(),
        Some("openai")
    );
    assert_eq!(
        rig.fixture.state.litellm_params.get("litellm_call_id"),
        Some(&text("call-9"))
    );
    assert_eq!(rig.fixture.logged("api_base"), Some(&text(&target)));
    let snapshot = rig.fixture.snapshot().expect("the logger saw the call");
    let Json::Object(fields) = snapshot else {
        panic!("the callback snapshot is an object");
    };
    assert_eq!(
        fields.get("eval_id"),
        Some(&text("eval_1")),
        "the logging update ran before the pre-call"
    );
    assert_eq!(fields.get("custom_llm_provider"), Some(&text("openai")));
    let Some(Json::Object(optional)) = fields.get("optional_params") else {
        panic!("optional_params is an object");
    };
    assert_eq!(optional.get("eval_id"), Some(&text("eval_1")));
}

#[tokio::test]
async fn delete_eval_defaults_the_provider_to_openai() {
    let serving = json_server("200 OK", DELETED, Duration::ZERO);
    let base = url(&serving);
    let mut rig = Rig::new(
        RecordingLookup::new(base.clone()),
        TransportLimits::DEFAULT,
        REQUEST_TIMEOUT,
    );
    let params = openai_kwargs("https://records.example");
    let request = call("eval_1", &params);
    let _answer = run(&mut rig, Operation::Delete, &request)
        .await
        .expect("the fixture answers");
    serving.finish().expect("the fixture finishes");
    assert_eq!(
        rig.configs.recorder().providers(),
        vec!["openai".to_owned()]
    );
    assert_eq!(
        rig.fixture.state.custom_llm_provider.as_deref(),
        Some("openai")
    );
}

#[tokio::test]
async fn delete_eval_names_the_operation_for_an_unsupported_provider() {
    let serving = json_server("200 OK", DELETED, Duration::ZERO);
    let mut rig = Rig::new(RealConfigs, TransportLimits::DEFAULT, REQUEST_TIMEOUT);
    let params = openai_kwargs("https://api.example");
    let request = EvalLifecycleCall {
        custom_llm_provider: Some("mistral"),
        ..call("eval_1", &params)
    };
    let failure = run(&mut rig, Operation::Delete, &request)
        .await
        .expect_err("mistral has no evals API");
    let ErrorClass::BadRequest(details) = &failure.class else {
        panic!("a bad request, got {:?}", failure.class);
    };
    assert_eq!(details.status.0, 400);
    assert_eq!(details.provider.as_deref(), Some("mistral"));
    assert_eq!(details.model.as_deref(), Some(""));
    assert_eq!(details.message, "DELETE eval is not supported for mistral");
    assert_no_request(serving);
}

#[tokio::test]
async fn delete_eval_accepts_extra_query_without_sending_it() {
    let serving = json_server("200 OK", DELETED, Duration::ZERO);
    let mut rig = Rig::new(RealConfigs, TransportLimits::DEFAULT, REQUEST_TIMEOUT);
    let params = openai_kwargs(&url(&serving));
    let query = object(r#"{"force":"1"}"#);
    let request = EvalLifecycleCall {
        extra_query: Some(&query),
        ..call("eval_1", &params)
    };
    let _answer = run(&mut rig, Operation::Delete, &request)
        .await
        .expect("the fixture answers");
    let seen = serving.finish().expect("the fixture finishes");
    assert_line(&seen, "DELETE", "/v1/evals/eval_1");
    assert!(
        !seen.line.contains('?'),
        "no query is sent, and the request line is {}",
        seen.line
    );
}

#[tokio::test]
async fn adelete_eval_answers_like_delete_eval() {
    let serving = json_server("200 OK", DELETED, Duration::ZERO);
    let mut rig = Rig::new(RealConfigs, TransportLimits::DEFAULT, REQUEST_TIMEOUT);
    let params = openai_kwargs(&url(&serving));
    let request = call("eval_1", &params);
    let answer = run(&mut rig, Operation::AsyncDelete, &request)
        .await
        .expect("the fixture answers");
    let response = deleted(answer);
    let seen = serving.finish().expect("the fixture finishes");
    assert_line(&seen, "DELETE", "/v1/evals/eval_1");
    assert_eq!(response.eval_id, "eval_1");
    assert_eq!(
        params.get("adelete_eval"),
        None,
        "the flag goes on a copy, so the caller's keywords keep their keys"
    );
}

// adelete_eval hands the caller's eval id, headers, api base and timeout to the same body
#[tokio::test]
async fn adelete_eval_forwards_the_caller_arguments() {
    let serving = json_server("200 OK", DELETED, Duration::from_millis(300));
    let base = url(&serving);
    let mut rig = Rig::new(
        RecordingLookup::new(base),
        TransportLimits::DEFAULT,
        REQUEST_TIMEOUT,
    );
    let params = kwargs(&[
        ("api_key", text("sk-test")),
        ("api_base", text("https://records.example")),
    ]);
    let extra = headers(&[("x-extra", "one")]);
    let request = EvalLifecycleCall {
        extra_headers: Some(&extra),
        timeout: Some(Duration::from_millis(50)),
        ..call("eval_7", &params)
    };
    let failure = run(&mut rig, Operation::AsyncDelete, &request)
        .await
        .expect_err("the 50 ms timeout ends the call");
    let error = provider(&failure);
    assert_eq!(error.status_code, 500);
    assert!(error.status_code_is_synthesized);
    assert!(matches!(&failure.class, ErrorClass::InternalServer(_)));
    serving
        .finish()
        .expect("the fixture read the request before its pause");
    let asked = rig.configs.recorder().only_call();
    assert_eq!(asked.eval_id, "eval_7");
    assert_eq!(asked.api_base, "https://records.example");
    assert_eq!(
        asked.headers.get("x-extra").map(String::as_str),
        Some("one")
    );
}

// adelete_eval hands the caller's provider to the lookup, so the unsupported answer names mistral and nothing is sent
#[tokio::test]
async fn adelete_eval_names_the_caller_provider_for_an_unsupported_provider() {
    let serving = json_server("200 OK", DELETED, Duration::ZERO);
    let base = url(&serving);
    let mut rig = Rig::new(
        RecordingLookup::new(base),
        TransportLimits::DEFAULT,
        REQUEST_TIMEOUT,
    );
    let params = openai_kwargs("https://records.example");
    let request = EvalLifecycleCall {
        custom_llm_provider: Some("mistral"),
        ..call("eval_1", &params)
    };
    let failure = run(&mut rig, Operation::AsyncDelete, &request)
        .await
        .expect_err("mistral has no evals API");
    let ErrorClass::BadRequest(details) = &failure.class else {
        panic!("a bad request, got {:?}", failure.class);
    };
    assert_eq!(details.status.0, 400);
    assert_eq!(details.message, "DELETE eval is not supported for mistral");
    assert_eq!(
        rig.configs.recorder().providers(),
        vec!["mistral".to_owned()]
    );
    assert_no_request(serving);
}

// kwargs.pop("adelete_eval", False) runs before GenericLiteLLMParams, so the flag never reaches the provider config
#[tokio::test]
async fn delete_eval_pops_the_async_flag_from_the_parameters() {
    let serving = json_server("200 OK", DELETED, Duration::ZERO);
    let base = url(&serving);
    let mut rig = Rig::new(
        RecordingLookup::new(base.clone()),
        TransportLimits::DEFAULT,
        REQUEST_TIMEOUT,
    );
    let params = kwargs(&[
        ("api_key", text("sk-test")),
        ("api_base", text("https://records.example")),
        ("adelete_eval", Json::Bool(true)),
    ]);
    let request = call("eval_1", &params);
    let _answer = run(&mut rig, Operation::Delete, &request)
        .await
        .expect("the fixture answers");
    serving.finish().expect("the fixture finishes");
    let asked = rig.configs.recorder().only_call();
    assert_eq!(
        asked.params.get("adelete_eval"),
        None,
        "the async flag never reaches the parameters"
    );
    assert_eq!(
        asked.params.get("api_key").and_then(|value| value.as_str()),
        Some("sk-test")
    );
}

#[tokio::test]
async fn delete_eval_sends_the_callers_timeout() {
    let serving = json_server("200 OK", DELETED, Duration::from_millis(300));
    let mut rig = Rig::new(RealConfigs, TransportLimits::DEFAULT, REQUEST_TIMEOUT);
    let params = openai_kwargs(&url(&serving));
    let request = EvalLifecycleCall {
        timeout: Some(Duration::from_millis(50)),
        ..call("eval_1", &params)
    };
    let failure = run(&mut rig, Operation::Delete, &request)
        .await
        .expect_err("the 50 ms timeout ends the call");
    let error = provider(&failure);
    assert_eq!(error.status_code, 500);
    assert!(error.status_code_is_synthesized);
    assert_eq!(
        error.message,
        "the request ran past its read timeout or total deadline"
    );
    assert!(matches!(&failure.class, ErrorClass::InternalServer(_)));
    let seen = serving
        .finish()
        .expect("the fixture read the request before its pause");
    assert_line(&seen, "DELETE", "/v1/evals/eval_1");
}

#[tokio::test]
async fn delete_eval_falls_back_to_the_request_timeout() {
    let serving = json_server("200 OK", DELETED, Duration::from_millis(300));
    let mut rig = Rig::new(
        RealConfigs,
        TransportLimits::DEFAULT,
        Duration::from_millis(50),
    );
    let params = openai_kwargs(&url(&serving));
    let request = call("eval_1", &params);
    let failure = run(&mut rig, Operation::Delete, &request)
        .await
        .expect_err("the 50 ms request timeout ends the call");
    let error = provider(&failure);
    assert_eq!(error.status_code, 500);
    assert_eq!(
        error.message,
        "the request ran past its read timeout or total deadline"
    );
    serving
        .finish()
        .expect("the fixture read the request before its pause");
}

#[tokio::test]
async fn delete_eval_treats_a_zero_timeout_as_unset() {
    let serving = json_server("200 OK", DELETED, Duration::from_millis(300));
    let mut rig = Rig::new(
        RealConfigs,
        TransportLimits::DEFAULT,
        Duration::from_millis(50),
    );
    let params = openai_kwargs(&url(&serving));
    let request = EvalLifecycleCall {
        timeout: Some(Duration::ZERO),
        ..call("eval_1", &params)
    };
    let failure = run(&mut rig, Operation::Delete, &request)
        .await
        .expect_err("a zero timeout takes the 50 ms request timeout");
    let error = provider(&failure);
    assert_eq!(
        error.message,
        "the request ran past its read timeout or total deadline"
    );
    serving
        .finish()
        .expect("the fixture read the request before its pause");
}
