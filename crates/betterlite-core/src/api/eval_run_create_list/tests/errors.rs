use std::time::Duration;

use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;
use betterlite_openai::evals::error::EvalsError;

use super::support::{
    Failure, Recorder, RecordingConfig, create_args, error, headers, json, kwargs, list_args,
    object, sync_create, sync_list,
};
use crate::api::eval_run_create_list::args::{CreateRunArgs, ListRunsArgs};
use crate::api::eval_run_create_list::error::EvalRunCallError;

// ProviderConfigManager.get_provider_evals_api_config returns None for a provider without evals support
#[test]
fn unsupported_create_provider_is_a_bad_request() {
    let params = kwargs("{}");
    let data_source = object(r#"{"type":"jsonl"}"#);
    let request = CreateRunArgs {
        custom_llm_provider: Some("azure"),
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
        EvalRunCallError::UnsupportedCreate { .. }
    ));
    assert_eq!(
        failure.error.to_string(),
        "CREATE run is not supported for azure"
    );
    let ErrorClass::BadRequest(details) = &failure.class else {
        panic!("expected a bad request, got {:?}", failure.class);
    };
    assert_eq!(details.status.0, 400);
    assert_eq!(details.provider.as_deref(), Some("azure"));
    assert_eq!(details.model.as_deref(), Some(""));
    assert!(calls.transport.creates().is_empty());
}

#[test]
fn unsupported_list_provider_is_a_bad_request() {
    let params = kwargs("{}");
    let request = ListRunsArgs {
        custom_llm_provider: Some("azure"),
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
        EvalRunCallError::UnsupportedList { .. }
    ));
    assert_eq!(
        failure.error.to_string(),
        "LIST runs is not supported for azure"
    );
    let ErrorClass::BadRequest(details) = &failure.class else {
        panic!("expected a bad request, got {:?}", failure.class);
    };
    assert_eq!(details.status.0, 400);
    assert_eq!(details.provider.as_deref(), Some("azure"));
    assert_eq!(details.model.as_deref(), Some(""));
    assert!(calls.transport.lists().is_empty());
}

// litellm.LlmProviders(name) raises its ValueError before any config lookup happens
#[test]
fn unknown_provider_is_a_bad_request() {
    let params = kwargs("{}");
    let data_source = object(r#"{"type":"jsonl"}"#);
    let request = CreateRunArgs {
        custom_llm_provider: Some("not-a-provider"),
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
        EvalRunCallError::UnknownProvider { .. }
    ));
    assert_eq!(
        failure.error.to_string(),
        "'not-a-provider' is not a valid LlmProviders"
    );
    let ErrorClass::BadRequest(details) = &failure.class else {
        panic!("expected a bad request, got {:?}", failure.class);
    };
    assert_eq!(details.status.0, 400);
    assert_eq!(details.model.as_deref(), Some(""));
    assert_eq!(details.provider.as_deref(), Some("not-a-provider"));
    assert!(calls.transport.creates().is_empty());
}

// list_runs goes through the same LlmProviders lookup, so an unknown name never reaches the provider
#[test]
fn list_rejects_an_unknown_provider_name() {
    let params = kwargs("{}");
    let request = ListRunsArgs {
        custom_llm_provider: Some("not-a-provider"),
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
        EvalRunCallError::UnknownProvider { .. }
    ));
    assert_eq!(
        failure.error.to_string(),
        "'not-a-provider' is not a valid LlmProviders"
    );
    let ErrorClass::BadRequest(details) = &failure.class else {
        panic!("expected a bad request, got {:?}", failure.class);
    };
    assert_eq!(details.status.0, 400);
    assert_eq!(details.model.as_deref(), Some(""));
    assert_eq!(details.provider.as_deref(), Some("not-a-provider"));
    assert!(calls.transport.lists().is_empty());
}

#[test]
fn create_maps_a_provider_rate_limit() {
    let params = kwargs("{}");
    let data_source = object(r#"{"type":"jsonl"}"#);
    let request = create_args("eval-1", &data_source, &params);

    let (answer, _fixture, _calls) = sync_create(
        RecordingConfig::with_key("sk-test"),
        Recorder::failing(Failure::Provider),
        request,
    );

    let failure = error(answer);
    assert!(matches!(&failure.error, EvalRunCallError::Transport(_)));
    let ErrorClass::RateLimit(details) = &failure.class else {
        panic!("expected a rate limit, got {:?}", failure.class);
    };
    assert_eq!(details.context.provider.as_deref(), Some("openai"));
    assert_eq!(details.context.model.as_deref(), Some(""));
    assert!(details.context.message.contains("rate limited"));
}

#[test]
fn list_maps_a_provider_rate_limit() {
    let params = kwargs("{}");
    let request = list_args("eval-1", &params);

    let (answer, _fixture, _calls) = sync_list(
        RecordingConfig::with_key("sk-test"),
        Recorder::failing(Failure::Provider),
        request,
    );

    let failure = error(answer);
    let ErrorClass::RateLimit(details) = &failure.class else {
        panic!("expected a rate limit, got {:?}", failure.class);
    };
    assert_eq!(details.context.provider.as_deref(), Some("openai"));
    assert!(details.context.message.contains("rate limited"));
}

// a handler failure outside the provider family has no status, so exception_type reports api_connection
#[test]
fn a_local_handler_error_is_an_api_connection() {
    let params = kwargs("{}");
    let data_source = object(r#"{"type":"jsonl"}"#);
    let request = create_args("eval-1", &data_source, &params);

    let (answer, _fixture, _calls) = sync_create(
        RecordingConfig::with_key("sk-test"),
        Recorder::failing(Failure::Local),
        request,
    );

    let failure = error(answer);
    assert!(matches!(&failure.error, EvalRunCallError::Transport(_)));
    let ErrorClass::ApiConnection(context) = &failure.class else {
        panic!("expected an api connection, got {:?}", failure.class);
    };
    assert_eq!(context.model.as_deref(), Some(""));
    assert_eq!(context.provider.as_deref(), Some("openai"));
}

// the blocking entry polls once: a handler still waiting when it polled is reported, not waited on
#[test]
fn a_sync_call_that_is_still_pending_is_suspended() {
    let params = kwargs("{}");
    let data_source = object(r#"{"type":"jsonl"}"#);
    let request = create_args("eval-1", &data_source, &params);

    let (answer, _fixture, calls) = sync_create(
        RecordingConfig::with_key("sk-test"),
        Recorder::failing(Failure::Deferred),
        request,
    );

    let failure = error(answer);
    assert!(matches!(&failure.error, EvalRunCallError::Suspended));
    let ErrorClass::ApiConnection(context) = &failure.class else {
        panic!("expected an api connection, got {:?}", failure.class);
    };
    assert_eq!(context.model.as_deref(), Some(""));
    assert_eq!(calls.transport.creates().len(), 1);
}

// validate_environment runs before the transform and the handler, so a missing key never reaches the wire
#[test]
fn create_without_a_key_fails_before_the_transport() {
    let params = kwargs("{}");
    let data_source = object(r#"{"type":"jsonl"}"#);
    let request = create_args("eval-1", &data_source, &params);

    let (answer, _fixture, calls) =
        sync_create(RecordingConfig::without_key(), Recorder::new(), request);

    let failure = error(answer);
    assert!(matches!(
        &failure.error,
        EvalRunCallError::Config(EvalsError::ApiKeyRequired)
    ));
    let ErrorClass::ApiConnection(context) = &failure.class else {
        panic!("expected an api connection, got {:?}", failure.class);
    };
    assert_eq!(context.provider.as_deref(), Some("openai"));
    assert!(calls.transport.creates().is_empty());
}

// local_vars = locals() at create_run's entry: every argument, the caller's kwargs, and null for absent ones
#[test]
fn create_local_vars_hold_every_argument() {
    let params = kwargs(r#"{"api_key":"sk-test"}"#);
    let data_source = object(r#"{"type":"jsonl"}"#);
    let metadata = object(r#"{"team":"evals"}"#);
    let extra_headers = headers(&[("X-Extra", "1")]);
    let extra_query = object(r#"{"q":"x"}"#);
    let extra_body = object(r#"{"name":"nightly"}"#);
    let request = CreateRunArgs {
        name: Some("nightly"),
        metadata: Some(&metadata),
        extra_headers: Some(&extra_headers),
        extra_query: Some(&extra_query),
        extra_body: Some(&extra_body),
        timeout: Some(Duration::from_secs(30)),
        custom_llm_provider: Some("openai"),
        ..create_args("eval-1", &data_source, &params)
    };

    let (answer, _fixture, _calls) = sync_create(
        RecordingConfig::with_key("sk-test"),
        Recorder::failing(Failure::Provider),
        request,
    );

    let failure = error(answer);
    assert_eq!(
        Json::Object(failure.local_vars.as_ref().clone()),
        json(
            r#"{"eval_id":"eval-1","data_source":{"type":"jsonl"},"name":"nightly","metadata":{"team":"evals"},"extra_headers":{"X-Extra":"1"},"extra_query":{"q":"x"},"extra_body":{"name":"nightly"},"timeout":30.0,"custom_llm_provider":"openai","kwargs":{"api_key":"sk-test"}}"#
        )
    );
}

// the list entry's local_vars carry the four pagination values and no data_source
#[test]
fn list_local_vars_hold_every_argument() {
    let params = kwargs(r#"{"api_key":"sk-test"}"#);
    let extra_headers = headers(&[("X-Extra", "1")]);
    let extra_query = object(r#"{"q":"x"}"#);
    let request = ListRunsArgs {
        limit: Some(10),
        after: Some("run-0"),
        before: Some("run-9"),
        order: Some("desc"),
        extra_headers: Some(&extra_headers),
        extra_query: Some(&extra_query),
        timeout: Some(Duration::from_millis(1500)),
        custom_llm_provider: Some("openai"),
        ..list_args("eval-1", &params)
    };

    let (answer, _fixture, _calls) = sync_list(
        RecordingConfig::with_key("sk-test"),
        Recorder::failing(Failure::Provider),
        request,
    );

    let failure = error(answer);
    assert_eq!(
        Json::Object(failure.local_vars.as_ref().clone()),
        json(
            r#"{"eval_id":"eval-1","limit":10,"after":"run-0","before":"run-9","order":"desc","extra_headers":{"X-Extra":"1"},"extra_query":{"q":"x"},"timeout":1.5,"custom_llm_provider":"openai","kwargs":{"api_key":"sk-test"}}"#
        )
    );
}
