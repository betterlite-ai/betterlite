use std::time::Duration;

use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;
use rstest::rstest;

use super::support::{
    Recorder, RecordingConfig, create_args, error, header, headers, json, kwargs, object,
    run_fixture, sync_create,
};
use crate::api::eval_run_create_list::args::CreateRunArgs;
use crate::api::eval_run_create_list::error::EvalRunCallError;

#[test]
fn create_sends_the_data_source_and_name() {
    let params = kwargs("{}");
    let data_source = object(r#"{"type":"jsonl","source":{"type":"file_id","id":"file-1"}}"#);
    let request = CreateRunArgs {
        name: Some("nightly"),
        ..create_args("eval-1", &data_source, &params)
    };

    let (answer, _fixture, calls) = sync_create(
        RecordingConfig::with_key("sk-test"),
        Recorder::new(),
        request,
    );

    assert_eq!(answer.expect("the call answers"), run_fixture());
    let creates = calls.transport.creates();
    let seen = creates.first().expect("the create reached the transport");
    assert_eq!(seen.provider, "openai");
    assert_eq!(seen.url, "https://api.openai.com/v1/evals/eval-1/runs");
    assert_eq!(
        seen.body,
        object(
            r#"{"data_source":{"type":"jsonl","source":{"type":"file_id","id":"file-1"}},"name":"nightly"}"#
        )
    );
}

#[test]
fn create_omits_an_absent_name() {
    let params = kwargs("{}");
    let data_source = object(r#"{"type":"jsonl"}"#);
    let request = create_args("eval-1", &data_source, &params);

    let (answer, _fixture, calls) = sync_create(
        RecordingConfig::with_key("sk-test"),
        Recorder::new(),
        request,
    );

    assert_eq!(answer.expect("the call answers"), run_fixture());
    let creates = calls.transport.creates();
    let seen = creates.first().expect("the create reached the transport");
    assert_eq!(seen.body, object(r#"{"data_source":{"type":"jsonl"}}"#));
}

// LiteLLM comments the metadata assignment out: only extra_body can put it in the request
#[test]
fn create_keeps_metadata_out_of_the_body() {
    let params = kwargs("{}");
    let data_source = object(r#"{"type":"jsonl"}"#);
    let metadata = object(r#"{"team":"evals"}"#);
    let request = CreateRunArgs {
        name: Some("nightly"),
        metadata: Some(&metadata),
        ..create_args("eval-1", &data_source, &params)
    };

    let (answer, _fixture, calls) = sync_create(
        RecordingConfig::with_key("sk-test"),
        Recorder::new(),
        request,
    );

    assert_eq!(answer.expect("the call answers"), run_fixture());
    let creates = calls.transport.creates();
    let seen = creates.first().expect("the create reached the transport");
    assert_eq!(
        seen.body,
        object(r#"{"data_source":{"type":"jsonl"},"name":"nightly"}"#)
    );
}

#[test]
fn create_applies_extra_body_data_source_and_name() {
    let params = kwargs("{}");
    let data_source = object(r#"{"type":"jsonl","source":{"type":"file_id","id":"file-1"}}"#);
    let extra_body = object(r#"{"data_source":{"type":"stored_completions"},"name":"replaced"}"#);
    let request = CreateRunArgs {
        name: Some("nightly"),
        extra_body: Some(&extra_body),
        ..create_args("eval-1", &data_source, &params)
    };

    let (answer, _fixture, calls) = sync_create(
        RecordingConfig::with_key("sk-test"),
        Recorder::new(),
        request,
    );

    assert_eq!(answer.expect("the call answers"), run_fixture());
    let creates = calls.transport.creates();
    let seen = creates.first().expect("the create reached the transport");
    assert_eq!(
        seen.body,
        object(r#"{"data_source":{"type":"stored_completions"},"name":"replaced"}"#)
    );
}

#[test]
fn create_extra_body_adds_metadata() {
    let params = kwargs("{}");
    let data_source = object(r#"{"type":"jsonl"}"#);
    let extra_body = object(r#"{"metadata":{"team":"evals"}}"#);
    let request = CreateRunArgs {
        extra_body: Some(&extra_body),
        ..create_args("eval-1", &data_source, &params)
    };

    let (answer, _fixture, calls) = sync_create(
        RecordingConfig::with_key("sk-test"),
        Recorder::new(),
        request,
    );

    assert_eq!(answer.expect("the call answers"), run_fixture());
    let creates = calls.transport.creates();
    let seen = creates.first().expect("the create reached the transport");
    assert_eq!(
        seen.body,
        object(r#"{"data_source":{"type":"jsonl"},"metadata":{"team":"evals"}}"#)
    );
}

#[test]
fn create_ignores_null_extra_body_keys() {
    let params = kwargs("{}");
    let data_source = object(r#"{"type":"jsonl"}"#);
    let extra_body = object(r#"{"name":null,"unknown":null}"#);
    let request = CreateRunArgs {
        name: Some("nightly"),
        extra_body: Some(&extra_body),
        ..create_args("eval-1", &data_source, &params)
    };

    let (answer, _fixture, calls) = sync_create(
        RecordingConfig::with_key("sk-test"),
        Recorder::new(),
        request,
    );

    assert_eq!(answer.expect("the call answers"), run_fixture());
    let creates = calls.transport.creates();
    let seen = creates.first().expect("the create reached the transport");
    assert_eq!(seen.body, object(r#"{"data_source":{"type":"jsonl"}}"#));
}

#[test]
fn create_refuses_an_unknown_extra_body_key() {
    let params = kwargs("{}");
    let data_source = object(r#"{"type":"jsonl"}"#);
    let extra_body = object(r#"{"temperature":0.5}"#);
    let request = CreateRunArgs {
        extra_body: Some(&extra_body),
        ..create_args("eval-1", &data_source, &params)
    };

    let (answer, _fixture, calls) = sync_create(
        RecordingConfig::with_key("sk-test"),
        Recorder::new(),
        request,
    );

    let failure = error(answer);
    assert!(
        matches!(&failure.error, EvalRunCallError::UnknownBodyKey { key } if key.as_str() == "temperature")
    );
    let ErrorClass::BadRequest(details) = &failure.class else {
        panic!("expected a bad request, got {:?}", failure.class);
    };
    assert_eq!(details.body_param.as_deref(), Some("temperature"));
    assert!(calls.transport.creates().is_empty());
}

#[rstest]
#[case::data_source_null(r#"{"data_source":null}"#, "data_source")]
#[case::data_source_text(r#"{"data_source":"items"}"#, "data_source")]
#[case::metadata_text(r#"{"metadata":"team"}"#, "metadata")]
#[case::name_number(r#"{"name":7}"#, "name")]
fn create_refuses_an_extra_body_value_of_the_wrong_type(#[case] extra: &str, #[case] field: &str) {
    let params = kwargs("{}");
    let data_source = object(r#"{"type":"jsonl"}"#);
    let extra_body = object(extra);
    let request = CreateRunArgs {
        extra_body: Some(&extra_body),
        ..create_args("eval-1", &data_source, &params)
    };

    let (answer, _fixture, calls) = sync_create(
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
    assert!(calls.transport.creates().is_empty());
}

#[test]
fn create_validates_headers_and_passes_them_to_the_transport() {
    let params = kwargs("{}");
    let data_source = object(r#"{"type":"jsonl"}"#);
    let extra = headers(&[("X-Extra", "1")]);
    let request = CreateRunArgs {
        extra_headers: Some(&extra),
        ..create_args("eval-1", &data_source, &params)
    };

    let (answer, _fixture, calls) = sync_create(
        RecordingConfig::with_key("sk-test"),
        Recorder::new(),
        request,
    );

    assert_eq!(answer.expect("the call answers"), run_fixture());
    let creates = calls.transport.creates();
    let seen = creates.first().expect("the create reached the transport");
    assert_eq!(header(&seen.headers, "X-Extra"), "1");
    assert_eq!(header(&seen.headers, "Authorization"), "Bearer sk-test");
    assert_eq!(header(&seen.headers, "Content-Type"), "application/json");
    let transforms = calls.providers.transforms();
    let transform = transforms.first().expect("the transform ran");
    assert_eq!(transform.operation, "create_run");
    assert_eq!(transform.eval_id, "eval-1");
    assert_eq!(transform.headers, seen.headers);
}

// GenericLiteLLMParams(**kwargs) hands the caller's api_key and api_base to validate and to the transform
#[test]
fn create_uses_the_kwargs_api_key_and_api_base() {
    let params = kwargs(r#"{"api_key":"sk-other","api_base":"https://proxy.example.com"}"#);
    let data_source = object(r#"{"type":"jsonl"}"#);
    let request = create_args("eval-1", &data_source, &params);

    let (answer, _fixture, calls) =
        sync_create(RecordingConfig::without_key(), Recorder::new(), request);

    assert_eq!(answer.expect("the call answers"), run_fixture());
    let creates = calls.transport.creates();
    let seen = creates.first().expect("the create reached the transport");
    assert_eq!(seen.url, "https://proxy.example.com/v1/evals/eval-1/runs");
    assert_eq!(header(&seen.headers, "Authorization"), "Bearer sk-other");
}

#[test]
fn create_defaults_the_timeout_to_600_seconds() {
    let params = kwargs("{}");
    let data_source = object(r#"{"type":"jsonl"}"#);
    let request = create_args("eval-1", &data_source, &params);

    let (answer, _fixture, calls) = sync_create(
        RecordingConfig::with_key("sk-test"),
        Recorder::new(),
        request,
    );

    assert_eq!(answer.expect("the call answers"), run_fixture());
    let creates = calls.transport.creates();
    let seen = creates.first().expect("the create reached the transport");
    assert_eq!(seen.timeout, Duration::from_secs(600));
}

#[test]
fn create_uses_a_supplied_timeout() {
    let params = kwargs("{}");
    let data_source = object(r#"{"type":"jsonl"}"#);
    let request = CreateRunArgs {
        timeout: Some(Duration::from_millis(1500)),
        ..create_args("eval-1", &data_source, &params)
    };

    let (answer, _fixture, calls) = sync_create(
        RecordingConfig::with_key("sk-test"),
        Recorder::new(),
        request,
    );

    assert_eq!(answer.expect("the call answers"), run_fixture());
    let creates = calls.transport.creates();
    let seen = creates.first().expect("the create reached the transport");
    assert_eq!(seen.timeout, Duration::from_millis(1500));
}

// `timeout or ...` keeps any supplied value, so an explicit zero is not replaced by the 600-second default
#[test]
fn create_uses_an_explicit_zero_timeout() {
    let params = kwargs("{}");
    let data_source = object(r#"{"type":"jsonl"}"#);
    let request = CreateRunArgs {
        timeout: Some(Duration::ZERO),
        ..create_args("eval-1", &data_source, &params)
    };

    let (answer, _fixture, calls) = sync_create(
        RecordingConfig::with_key("sk-test"),
        Recorder::new(),
        request,
    );

    assert_eq!(answer.expect("the call answers"), run_fixture());
    let creates = calls.transport.creates();
    let seen = creates.first().expect("the create reached the transport");
    assert_eq!(seen.timeout, Duration::ZERO);
}

#[test]
fn create_logs_the_body_as_optional_params() {
    let params = kwargs("{}");
    let data_source = object(r#"{"type":"jsonl","source":{"type":"file_id","id":"file-1"}}"#);
    let request = CreateRunArgs {
        name: Some("nightly"),
        ..create_args("eval-1", &data_source, &params)
    };

    let (answer, fixture, _calls) = sync_create(
        RecordingConfig::with_key("sk-test"),
        Recorder::new(),
        request,
    );

    assert_eq!(answer.expect("the call answers"), run_fixture());
    assert_eq!(
        fixture.detail("optional_params"),
        Some(&json(
            r#"{"data_source":{"type":"jsonl","source":{"type":"file_id","id":"file-1"}},"name":"nightly"}"#
        ))
    );
    assert_eq!(
        fixture.detail("custom_llm_provider"),
        Some(&Json::Text("openai".to_owned()))
    );
}

// a client or shared_session kwarg is carried in the log and local_vars, but the transport's pooled client sends the call
#[test]
fn create_ignores_a_caller_client_kwarg() {
    let params = kwargs(r#"{"client":"caller-client","shared_session":"caller-session"}"#);
    let data_source = object(r#"{"type":"jsonl"}"#);
    let request = create_args("eval-1", &data_source, &params);

    let (answer, _fixture, calls) = sync_create(
        RecordingConfig::with_key("sk-test"),
        Recorder::new(),
        request,
    );

    assert_eq!(answer.expect("the call answers"), run_fixture());
    let creates = calls.transport.creates();
    let seen = creates.first().expect("the create reached the transport");
    assert_eq!(seen.url, "https://api.openai.com/v1/evals/eval-1/runs");
    assert_eq!(seen.body, object(r#"{"data_source":{"type":"jsonl"}}"#));
}

// kwargs get("litellm_call_id", None) is recorded as null when the caller names no call id
#[test]
fn create_without_a_call_id_logs_null() {
    let params = kwargs("{}");
    let data_source = object(r#"{"type":"jsonl"}"#);
    let request = create_args("eval-1", &data_source, &params);

    let (answer, fixture, _calls) = sync_create(
        RecordingConfig::with_key("sk-test"),
        Recorder::new(),
        request,
    );

    assert_eq!(answer.expect("the call answers"), run_fixture());
    let Some(Json::Object(litellm_params)) = fixture.detail("litellm_params") else {
        panic!("the logged litellm params are an object");
    };
    assert_eq!(litellm_params.get("litellm_call_id"), Some(&Json::Null));
}
