use betterlite_models::decisions::Json as ModelJson;
use betterlite_openai::response_input::ResponsesInput;
use betterlite_prompts::arguments::json::Object;

use super::create_call;
use super::support::{
    Answer, Config, Dispatcher, NO_CALLBACKS, NO_RESOLVER, RESP_A, Reply, Source, Wire,
    config_with, create_deps, execute, execute_logged, headers, hooks, models, object, params,
    text_input,
};
use crate::api::responses_compact_http::error::Stage as FailStage;
use crate::api::responses_compact_http::service::ClientKind;
use crate::api::responses_create_http::config::CreateParams;
use crate::api::responses_create_http::error::{CreateError, Stage as ErrorStage};
use crate::api::responses_create_http::service::CreateOutcome;

#[derive(Clone, Copy)]
enum FailureCase {
    Status,
    StreamingStatus,
    Step(FailStage),
    ContextUpdate,
    Build(BuildFailure),
}

#[derive(Clone, Copy)]
enum BuildFailure {
    Url,
    HeaderName,
    HeaderValue,
}

// Every failure the create transport converts or passes through, one after another
pub(super) async fn transport_failures() {
    let input = text_input("hello");
    let empty = Object::new();
    let call_params = params(r#"{"api_base":"http://mock/v1"}"#);
    let dispatcher = Dispatcher::quiet();
    for scenario in [
        FailureCase::Status,
        FailureCase::StreamingStatus,
        FailureCase::Step(FailStage::Transform),
        FailureCase::Step(FailStage::Normalize),
        FailureCase::Step(FailStage::Sign),
        FailureCase::Step(FailStage::Response),
        FailureCase::ContextUpdate,
        FailureCase::Build(BuildFailure::Url),
        FailureCase::Build(BuildFailure::HeaderName),
        FailureCase::Build(BuildFailure::HeaderValue),
    ] {
        transport_failure(scenario, &input, &empty, &call_params, &dispatcher).await;
    }
}

async fn transport_failure(
    scenario: FailureCase,
    input: &ResponsesInput,
    empty: &Object,
    call_params: &CreateParams,
    dispatcher: &Dispatcher,
) {
    match scenario {
        FailureCase::Status => {
            status_failure(input, empty, call_params, dispatcher, false).await;
        }
        FailureCase::StreamingStatus => {
            let streaming = object(r#"{"stream":true}"#);
            status_failure(input, &streaming, call_params, dispatcher, true).await;
        }
        FailureCase::Step(stage) => {
            step_failure(stage, input, empty, call_params, dispatcher).await;
        }
        FailureCase::ContextUpdate => context_update(input, call_params, dispatcher).await,
        FailureCase::Build(failure) => {
            build_failure(failure, call_params, dispatcher).await;
        }
    }
}

// A URL or header HTTP cannot take fails after pre_call, where LiteLLM's send would make it a synthesized 500
async fn build_failure(failure: BuildFailure, call_params: &CreateParams, dispatcher: &Dispatcher) {
    let input = text_input("hello");
    let empty = Object::new();
    let broken = params(r#"{"api_base":"not a url"}"#);
    let (base_params, extra_headers) = match failure {
        BuildFailure::Url => (&broken, headers(&[])),
        BuildFailure::HeaderName => (call_params, headers(&[("Bad Header", "x")])),
        BuildFailure::HeaderValue => (call_params, headers(&[("X-Trace", "bad\nvalue")])),
    };
    let config = Config::default();
    let wire = Wire::new(Vec::new());
    let source = Source::with(wire.clone());
    let mut call = create_call(&input, &empty, base_params);
    call.extra_headers = Some(&extra_headers);
    let create_hooks = hooks(NO_CALLBACKS, dispatcher, &NO_RESOLVER);
    let deps = create_deps(&config, &source);
    let logged = execute_logged(&deps, ClientKind::Sync, &call, &create_hooks).await;
    let Err(error) = logged.outcome else {
        panic!("the broken request fails");
    };
    match (failure, &error) {
        (BuildFailure::Url, CreateError::RequestUrl { .. })
        | (BuildFailure::HeaderName, CreateError::HeaderName { .. })
        | (BuildFailure::HeaderValue, CreateError::HeaderValue { .. }) => {}
        (BuildFailure::Url | BuildFailure::HeaderName | BuildFailure::HeaderValue, other) => {
            panic!("the failure is a request-build error: {other:?}");
        }
    }
    assert!(wire.requests().is_empty());
    assert_eq!(logged.details.get("input"), Some(&models("\"hello\"")));
    assert_eq!(logged.details.get("api_key"), Some(&models("\"\"")));
}

// HTTP 429 text=slow down with Retry-After=3 becomes B2402's 429 with the header preserved, streamed or not
async fn status_failure(
    input: &ResponsesInput,
    optional: &Object,
    call_params: &CreateParams,
    dispatcher: &Dispatcher,
    stream: bool,
) {
    let config = Config::default();
    let wire = Wire::new(vec![Answer::Json(
        Reply::json(429, "slow down").with_header("retry-after", "3"),
    )]);
    let source = Source::with(wire.clone());
    let call = create_call(input, optional, call_params);
    let create_hooks = hooks(NO_CALLBACKS, dispatcher, &NO_RESOLVER);
    let deps = create_deps(&config, &source);
    let Err(error) = execute(&deps, ClientKind::Sync, &call, &create_hooks).await else {
        panic!("the 429 fails");
    };
    let CreateError::Provider(provider) = error else {
        panic!("a 429 is a provider error")
    };
    assert_eq!(provider.status_code, 429);
    assert!(!provider.status_code_is_synthesized);
    assert_eq!(provider.message, "slow down");
    assert_eq!(provider.headers.get("retry-after"), Some(&models("\"3\"")));
    let sent = wire.requests();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].stream, stream);
}

// transform, normalize and sign fail before the send catch and send nothing; the response transform runs after it
async fn step_failure(
    stage: FailStage,
    input: &ResponsesInput,
    empty: &Object,
    call_params: &CreateParams,
    dispatcher: &Dispatcher,
) {
    let config = config_with(Some(stage), None);
    let wire = Wire::new(if stage == FailStage::Response {
        vec![Answer::Json(Reply::json(200, RESP_A))]
    } else {
        Vec::new()
    });
    let source = Source::with(wire.clone());
    let call = create_call(input, empty, call_params);
    let create_hooks = hooks(NO_CALLBACKS, dispatcher, &NO_RESOLVER);
    let deps = create_deps(&config, &source);
    let Err(error) = execute(&deps, ClientKind::Sync, &call, &create_hooks).await else {
        panic!("the step fails");
    };
    let message = error.to_string();
    let CreateError::Step { stage: found, .. } = error else {
        panic!("the failure is a step error: {error:?}");
    };
    assert_eq!(found, expected_stage(stage));
    assert_eq!(message, expected_message(stage));
    let sent = wire.requests();
    if stage == FailStage::Response {
        assert_eq!(sent.len(), 1);
    } else {
        assert!(sent.is_empty());
    }
}

fn expected_stage(stage: FailStage) -> ErrorStage {
    match stage {
        FailStage::Transform => ErrorStage::Transform,
        FailStage::Normalize => ErrorStage::Normalize,
        FailStage::Sign => ErrorStage::Sign,
        FailStage::Response => ErrorStage::Response,
        FailStage::Url | FailStage::Validate => panic!("only the transfer steps fail here"),
    }
}

fn expected_message(stage: FailStage) -> &'static str {
    match stage {
        FailStage::Transform => "request transform failed: the transform failed",
        FailStage::Normalize => "request normalization failed: the normalize failed",
        FailStage::Sign => "request signing failed: the sign failed",
        FailStage::Response => "response transform failed: the response transform failed",
        FailStage::Url | FailStage::Validate => panic!("only the transfer steps fail here"),
    }
}

// the optional map updates the request context first and generic params still land last
async fn context_update(
    input: &ResponsesInput,
    call_params: &CreateParams,
    dispatcher: &Dispatcher,
) {
    let optional = object(r#"{"stream":true,"input":"override","litellm_params":"ignored"}"#);
    let fake = models("true");
    let wire = Wire::new(vec![Answer::Json(Reply::json(200, RESP_A))]);
    let source = Source::with(wire);
    let mut call = create_call(input, &optional, call_params);
    call.fake_stream = Some(&fake);
    let config = Config::default();
    let create_hooks = hooks(NO_CALLBACKS, dispatcher, &NO_RESOLVER);
    let deps = create_deps(&config, &source);
    let outcome = execute(&deps, ClientKind::Sync, &call, &create_hooks)
        .await
        .expect("the context call");
    let CreateOutcome::Synthetic(synthetic) = outcome else {
        panic!("the fake call returns synthetic events")
    };
    let context = synthetic.setup.request_data.expect("the request context");
    assert_eq!(context.get("input"), Some(&models("\"override\"")));
    assert_eq!(context.get("stream"), Some(&models("true")));
    assert!(matches!(
        context.get("litellm_params"),
        Some(ModelJson::Object(_))
    ));
}
