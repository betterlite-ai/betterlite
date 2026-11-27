use std::time::Duration;

use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;
use rstest::rstest;

use super::support::{
    REQUEST_TIMEOUT, Recorder, RecordingConfig, error, header, headers, json, kwargs, list_args,
    object, runs_fixture, sync_list,
};
use crate::api::eval_run_create_list::args::ListRunsArgs;
use crate::api::eval_run_create_list::error::EvalRunCallError;

#[test]
fn list_sends_every_explicit_parameter() {
    let params = kwargs("{}");
    let request = ListRunsArgs {
        limit: Some(10),
        after: Some("run-0"),
        before: Some("run-9"),
        order: Some("asc"),
        ..list_args("eval-1", &params)
    };

    let (answer, _fixture, calls) = sync_list(
        RecordingConfig::with_key("sk-test"),
        Recorder::new(),
        request,
    );

    assert_eq!(answer.expect("the call answers"), runs_fixture());
    let lists = calls.transport.lists();
    let seen = lists.first().expect("the list reached the transport");
    assert_eq!(seen.provider, "openai");
    assert_eq!(seen.url, "https://api.openai.com/v1/evals/eval-1/runs");
    assert_eq!(
        seen.query,
        object(r#"{"limit":10,"after":"run-0","before":"run-9","order":"asc"}"#)
    );
}

#[test]
fn list_omits_absent_parameters() {
    let params = kwargs("{}");
    let request = list_args("eval-1", &params);

    let (answer, _fixture, calls) = sync_list(
        RecordingConfig::with_key("sk-test"),
        Recorder::new(),
        request,
    );

    assert_eq!(answer.expect("the call answers"), runs_fixture());
    let lists = calls.transport.lists();
    let seen = lists.first().expect("the list reached the transport");
    assert_eq!(seen.query, object("{}"));
}

// list_params.update(extra_query) runs after the explicit parameters, so extra_query wins
#[test]
fn list_extra_query_overrides_explicit_values() {
    let params = kwargs("{}");
    let extra_query = object(r#"{"limit":5,"after":"run-7","order":"desc"}"#);
    let request = ListRunsArgs {
        limit: Some(10),
        after: Some("run-0"),
        order: Some("asc"),
        extra_query: Some(&extra_query),
        ..list_args("eval-1", &params)
    };

    let (answer, _fixture, calls) = sync_list(
        RecordingConfig::with_key("sk-test"),
        Recorder::new(),
        request,
    );

    assert_eq!(answer.expect("the call answers"), runs_fixture());
    let lists = calls.transport.lists();
    let seen = lists.first().expect("the list reached the transport");
    assert_eq!(
        seen.query,
        object(r#"{"limit":5,"after":"run-7","order":"desc"}"#)
    );
}

// a null in extra_query overwrites the explicit value with None, and the transform drops it
#[test]
fn list_extra_query_null_clears_a_value() {
    let params = kwargs("{}");
    let extra_query = object(r#"{"limit":null,"after":null}"#);
    let request = ListRunsArgs {
        limit: Some(10),
        after: Some("run-0"),
        extra_query: Some(&extra_query),
        ..list_args("eval-1", &params)
    };

    let (answer, _fixture, calls) = sync_list(
        RecordingConfig::with_key("sk-test"),
        Recorder::new(),
        request,
    );

    assert_eq!(answer.expect("the call answers"), runs_fixture());
    let lists = calls.transport.lists();
    let seen = lists.first().expect("the list reached the transport");
    assert_eq!(seen.query, object("{}"));
}

// extra_query.update() would carry other keys into the dict, but ListRunsParams declares only the four
#[test]
fn list_extra_query_ignores_unknown_keys() {
    let params = kwargs("{}");
    let extra_query = object(r#"{"order_by":"created_at","expand":true}"#);
    let request = ListRunsArgs {
        extra_query: Some(&extra_query),
        ..list_args("eval-1", &params)
    };

    let (answer, _fixture, calls) = sync_list(
        RecordingConfig::with_key("sk-test"),
        Recorder::new(),
        request,
    );

    assert_eq!(answer.expect("the call answers"), runs_fixture());
    let lists = calls.transport.lists();
    let seen = lists.first().expect("the list reached the transport");
    assert_eq!(seen.query, object("{}"));
}

#[rstest]
#[case::limit_text(r#"{"limit":"ten"}"#, "limit")]
#[case::after_number(r#"{"after":5}"#, "after")]
#[case::order_number(r#"{"order":5}"#, "order")]
fn list_refuses_an_extra_query_value_of_the_wrong_type(#[case] extra: &str, #[case] field: &str) {
    let params = kwargs("{}");
    let extra_query = object(extra);
    let request = ListRunsArgs {
        extra_query: Some(&extra_query),
        ..list_args("eval-1", &params)
    };

    let (answer, _fixture, calls) = sync_list(
        RecordingConfig::with_key("sk-test"),
        Recorder::new(),
        request,
    );

    let failure = error(answer);
    assert!(matches!(
        &failure.error,
        EvalRunCallError::FieldType { field: named, .. } if *named == field
    ));
    let ErrorClass::BadRequest(details) = &failure.class else {
        panic!("expected a bad request, got {:?}", failure.class);
    };
    assert_eq!(details.body_param.as_deref(), Some(field));
    assert!(calls.transport.lists().is_empty());
}

#[test]
fn list_refuses_an_order_outside_the_literal() {
    let params = kwargs("{}");
    let request = ListRunsArgs {
        order: Some("sideways"),
        ..list_args("eval-1", &params)
    };

    let (answer, _fixture, calls) = sync_list(
        RecordingConfig::with_key("sk-test"),
        Recorder::new(),
        request,
    );

    let failure = error(answer);
    assert_eq!(failure.error.to_string(), "order must be asc or desc");
    let ErrorClass::BadRequest(details) = &failure.class else {
        panic!("expected a bad request, got {:?}", failure.class);
    };
    assert_eq!(details.body_param.as_deref(), Some("order"));
    assert!(calls.transport.lists().is_empty());
}

#[test]
fn list_validates_headers_and_passes_them_to_the_transport() {
    let params = kwargs("{}");
    let extra = headers(&[("X-Extra", "1")]);
    let request = ListRunsArgs {
        extra_headers: Some(&extra),
        ..list_args("eval-1", &params)
    };

    let (answer, _fixture, calls) = sync_list(
        RecordingConfig::with_key("sk-test"),
        Recorder::new(),
        request,
    );

    assert_eq!(answer.expect("the call answers"), runs_fixture());
    let lists = calls.transport.lists();
    let seen = lists.first().expect("the list reached the transport");
    assert_eq!(header(&seen.headers, "X-Extra"), "1");
    assert_eq!(header(&seen.headers, "Authorization"), "Bearer sk-test");
    assert_eq!(header(&seen.headers, "Content-Type"), "application/json");
    let transforms = calls.providers.transforms();
    let transform = transforms.first().expect("the transform ran");
    assert_eq!(transform.operation, "list_runs");
    assert_eq!(transform.eval_id, "eval-1");
    assert_eq!(transform.headers, seen.headers);
}

// GenericLiteLLMParams(**kwargs) hands the caller's api_key and api_base to validate and to the transform
#[test]
fn list_uses_the_kwargs_api_key_and_api_base() {
    let params = kwargs(r#"{"api_key":"sk-other","api_base":"https://proxy.example.com"}"#);
    let request = list_args("eval-1", &params);

    let (answer, _fixture, calls) =
        sync_list(RecordingConfig::without_key(), Recorder::new(), request);

    assert_eq!(answer.expect("the call answers"), runs_fixture());
    let lists = calls.transport.lists();
    let seen = lists.first().expect("the list reached the transport");
    assert_eq!(seen.url, "https://proxy.example.com/v1/evals/eval-1/runs");
    assert_eq!(header(&seen.headers, "Authorization"), "Bearer sk-other");
}

// `timeout or request_timeout`: REQUEST_TIMEOUT is the injected standard timeout, 6000 seconds
#[test]
fn list_defaults_the_timeout_to_the_request_timeout() {
    let params = kwargs("{}");
    let request = list_args("eval-1", &params);

    let (answer, _fixture, calls) = sync_list(
        RecordingConfig::with_key("sk-test"),
        Recorder::new(),
        request,
    );

    assert_eq!(answer.expect("the call answers"), runs_fixture());
    let lists = calls.transport.lists();
    let seen = lists.first().expect("the list reached the transport");
    assert_eq!(seen.timeout, REQUEST_TIMEOUT);
    assert_eq!(REQUEST_TIMEOUT, Duration::from_secs(6000));
}

#[test]
fn list_uses_a_supplied_timeout() {
    let params = kwargs("{}");
    let request = ListRunsArgs {
        timeout: Some(Duration::from_secs(2)),
        ..list_args("eval-1", &params)
    };

    let (answer, _fixture, calls) = sync_list(
        RecordingConfig::with_key("sk-test"),
        Recorder::new(),
        request,
    );

    assert_eq!(answer.expect("the call answers"), runs_fixture());
    let lists = calls.transport.lists();
    let seen = lists.first().expect("the list reached the transport");
    assert_eq!(seen.timeout, Duration::from_secs(2));
}

#[test]
fn list_logs_the_eval_id_and_the_query() {
    let params = kwargs("{}");
    let request = ListRunsArgs {
        limit: Some(10),
        after: Some("run-0"),
        ..list_args("eval-1", &params)
    };

    let (answer, fixture, _calls) = sync_list(
        RecordingConfig::with_key("sk-test"),
        Recorder::new(),
        request,
    );

    assert_eq!(answer.expect("the call answers"), runs_fixture());
    assert_eq!(
        fixture.detail("optional_params"),
        Some(&json(r#"{"eval_id":"eval-1","limit":10,"after":"run-0"}"#))
    );
    assert_eq!(
        fixture.detail("custom_llm_provider"),
        Some(&Json::Text("openai".to_owned()))
    );
}

// optional_params={"eval_id": eval_id, **query_params} keeps the eval id when no pagination is given
#[test]
fn list_logs_the_eval_id_without_pagination() {
    let params = kwargs("{}");
    let request = list_args("eval-1", &params);

    let (answer, fixture, _calls) = sync_list(
        RecordingConfig::with_key("sk-test"),
        Recorder::new(),
        request,
    );

    assert_eq!(answer.expect("the call answers"), runs_fixture());
    assert_eq!(
        fixture.detail("optional_params"),
        Some(&json(r#"{"eval_id":"eval-1"}"#))
    );
}
