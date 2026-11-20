#![cfg(test)]

mod support;

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::env_integer::timeout::RequestTimeout;
use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_shapes::DeleteResponseResult;
use betterlite_openai::response_resources::request::{ResourceOperation, resource_request};
use betterlite_openai::response_resources::response::TypedReply;
use http::Method;
use serde_json::Number;

use self::support::{
    CANCELLED, DELETED, Harness, Source, Wire, body_json, call as lifecycle_call, credentials,
    extra, header, json, openai_config, state as logging_state,
};
use crate::api::response_id_codecs::response::build_response_id;
use crate::api::responses_lifecycle_write::call::{
    LifecycleOperation, ResponsesLifecycleWriteCall,
};
use crate::api::responses_lifecycle_write::error::{
    LifecycleWriteError, ResponsesLifecycleWriteError,
};
use crate::api::responses_lifecycle_write::service::ResponsesLifecycleWrite;
use crate::api::responses_write_http::config::OpenAiResponsesConfig;
use crate::api::responses_write_http::error::WriteError;
use crate::api::responses_write_http::service::{
    ClientKind, SuppliedClient, WriteDeps, WriteRequest,
};

const BASE: &str = "http://mock.test/v1";

fn configured(environment: &[(&str, &str)]) -> RequestTimeout {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    RequestTimeout::resolve(&lookup).expect("the timeout fixture resolves")
}

fn writing<'deps, 'settings>(
    config: &'deps OpenAiResponsesConfig<'settings>,
    source: &'deps Source,
    request_timeout: RequestTimeout,
) -> ResponsesLifecycleWrite<'deps, OpenAiResponsesConfig<'settings>, Source> {
    ResponsesLifecycleWrite {
        deps: WriteDeps {
            config,
            source,
            supplied: None,
            shared_session: None,
        },
        request_timeout,
    }
}

async fn deleted(
    service: &ResponsesLifecycleWrite<'_, OpenAiResponsesConfig<'_>, Source>,
    call: &ResponsesLifecycleWriteCall,
    state: &mut LoggingState,
    env: &PrePostEnv<'_>,
    asynchronous: bool,
) -> Result<DeleteResponseResult, ResponsesLifecycleWriteError> {
    if asynchronous {
        service.adelete_responses(call, state, env).await
    } else {
        service.delete_responses(call, state, env).await
    }
}

async fn cancelled(
    service: &ResponsesLifecycleWrite<'_, OpenAiResponsesConfig<'_>, Source>,
    call: &ResponsesLifecycleWriteCall,
    state: &mut LoggingState,
    env: &PrePostEnv<'_>,
    asynchronous: bool,
) -> Result<TypedReply, ResponsesLifecycleWriteError> {
    if asynchronous {
        service.acancel_responses(call, state, env).await
    } else {
        service.cancel_responses(call, state, env).await
    }
}

fn configured_call(
    operation: LifecycleOperation,
    response_id: &str,
) -> ResponsesLifecycleWriteCall {
    let mut prepared = lifecycle_call(operation, response_id);
    prepared.params.api_base = Some(String::from(BASE));
    prepared
}

fn assert_wire(request: &WriteRequest, method: Method, path: &str) {
    assert_eq!(request.method, method);
    assert_eq!(request.url.as_str(), format!("{BASE}/responses/{path}"));
    assert!(request.url.query().is_none());
}

fn assert_api_connection(class: &ErrorClass) {
    let ErrorClass::ApiConnection(_) = class else {
        panic!("the ValueError becomes an APIConnectionError");
    };
    let metadata = class.metadata();
    assert_eq!(metadata.status_code, 500);
    assert_eq!(metadata.error_type, WireField::Null);
    assert_eq!(metadata.param, WireField::Null);
    assert_eq!(metadata.openai_code, None);
}

#[tokio::test]
async fn delete_and_cancel_encoded_provider() {
    let settings = credentials();
    let config = openai_config(&settings);
    let harness = Harness::new();
    let encoded = build_response_id(Some("openai"), Some("model-a"), "resp/a");
    let metadata = json(r#"{"model_info":{"id":"deployment_a"}}"#);
    for asynchronous in [false, true] {
        let wire = Wire::returning(200, DELETED);
        let source = Source::new(wire.clone());
        let service = writing(&config, &source, configured(&[]));
        let mut state = logging_state("gpt-5", "call-1");
        let env = harness.pre_post();
        let mut prepared = configured_call(LifecycleOperation::Delete, &encoded);
        prepared.custom_llm_provider = Some(String::from("anthropic"));
        prepared.params.litellm_metadata = Some(metadata.clone());
        let answer = deleted(&service, &prepared, &mut state, &env, asynchronous)
            .await
            .expect("the encoded provider wins over the explicit one");
        assert_eq!(answer.id.as_deref(), Some("resp_a"));
        assert_eq!(answer.object.as_deref(), Some("response"));
        assert_eq!(answer.deleted, Some(true));
        let delete_requests = wire.requests();
        assert_eq!(delete_requests.len(), 1);
        assert_wire(&delete_requests[0], Method::DELETE, "resp%2Fa");
        assert!(matches!(&delete_requests[0].body, RequestBody::Empty));
        drop(delete_requests);

        let cancel_wire = Wire::returning(200, CANCELLED);
        let cancel_source = Source::new(cancel_wire.clone());
        let cancel_service = writing(&config, &cancel_source, configured(&[]));
        let mut cancel_state = logging_state("gpt-5", "call-2");
        let mut cancel_call = configured_call(LifecycleOperation::Cancel, &encoded);
        cancel_call.custom_llm_provider = Some(String::from("anthropic"));
        cancel_call.params.litellm_metadata = Some(metadata.clone());
        let reply = cancelled(
            &cancel_service,
            &cancel_call,
            &mut cancel_state,
            &env,
            asynchronous,
        )
        .await
        .expect("the encoded provider wins for the cancel too");
        assert_eq!(reply.response.id, "resp_a");
        assert_eq!(reply.response.status.as_deref(), Some("cancelled"));
        assert_eq!(reply.response.model.as_deref(), Some("gpt-5"));
        let cancel_requests = cancel_wire.requests();
        assert_eq!(cancel_requests.len(), 1);
        assert_wire(&cancel_requests[0], Method::POST, "resp%2Fa/cancel");
        assert_eq!(body_json(&cancel_requests[0]), json("{}"));
        drop(cancel_requests);
    }
    let plain_wire = Wire::returning(200, DELETED);
    let plain_source = Source::new(plain_wire.clone());
    let plain_service = writing(&config, &plain_source, configured(&[]));
    let mut plain_state = logging_state("gpt-5", "call-3");
    let env = harness.pre_post();
    let mut plain_call = configured_call(LifecycleOperation::Delete, "resp_a");
    plain_call.custom_llm_provider = Some(String::from("openai"));
    deleted(&plain_service, &plain_call, &mut plain_state, &env, false)
        .await
        .expect("a raw id with an explicit openai provider succeeds");
    assert_wire(&plain_wire.requests()[0], Method::DELETE, "resp_a");
    let empty_provider = build_response_id(Some(""), Some("model-a"), "resp_b");
    let mut provider_call = configured_call(LifecycleOperation::Delete, &empty_provider);
    provider_call.custom_llm_provider = Some(String::from("openai"));
    deleted(
        &plain_service,
        &provider_call,
        &mut plain_state,
        &env,
        false,
    )
    .await
    .expect("an empty decoded provider falls back to the argument");
    assert_wire(&plain_wire.requests()[1], Method::DELETE, "resp_b");
    let empty_id = build_response_id(Some("openai"), Some("model-a"), "");
    let mut id_call = configured_call(LifecycleOperation::Delete, &empty_id);
    id_call.custom_llm_provider = Some(String::from("openai"));
    deleted(&plain_service, &id_call, &mut plain_state, &env, false)
        .await
        .expect("an empty decoded id falls back to the argument");
    let expected = resource_request(
        &empty_id,
        &format!("{BASE}/responses"),
        ResourceOperation::Delete,
    )
    .expect("the fallback id encodes");
    assert_eq!(plain_wire.requests()[2].url.as_str(), expected.url);
}

#[tokio::test]
async fn lifecycle_write_ignored_extras() {
    let settings = credentials();
    let config = openai_config(&settings);
    let harness = Harness::new();
    let trace: BTreeMap<String, String> = [("X-Trace".to_owned(), "kept".to_owned())]
        .into_iter()
        .collect();
    let mut delete_call = configured_call(LifecycleOperation::Delete, "resp_a");
    delete_call.custom_llm_provider = Some(String::from("openai"));
    delete_call.extra_headers = Some(trace.clone());
    delete_call.extra_query = Some(extra(r#"{"trace":"ignored"}"#));
    delete_call.extra_body = Some(extra(r#"{"reason":"ignored"}"#));
    let wire = Wire::returning(200, DELETED);
    let source = Source::new(wire.clone());
    let extras_service = writing(&config, &source, configured(&[]));
    let mut state = logging_state("gpt-5", "call-1");
    let env = harness.pre_post();
    deleted(&extras_service, &delete_call, &mut state, &env, false)
        .await
        .expect("the extras are accepted and dropped");
    let delete_requests = wire.requests();
    assert_eq!(delete_requests.len(), 1);
    assert_wire(&delete_requests[0], Method::DELETE, "resp_a");
    assert_eq!(
        header(&delete_requests[0], "x-trace").as_deref(),
        Some("kept")
    );
    assert!(matches!(&delete_requests[0].body, RequestBody::Empty));
    drop(delete_requests);

    let cancel_wire = Wire::returning(200, CANCELLED);
    let cancel_source = Source::new(cancel_wire.clone());
    let cancel_service = writing(&config, &cancel_source, configured(&[]));
    let mut cancel_call = configured_call(LifecycleOperation::Cancel, "resp_a");
    cancel_call.custom_llm_provider = Some(String::from("openai"));
    cancel_call.extra_headers = Some(trace);
    cancel_call.extra_query = Some(extra(r#"{"trace":"ignored"}"#));
    cancel_call.extra_body = Some(extra(r#"{"reason":"ignored"}"#));
    cancelled(&cancel_service, &cancel_call, &mut state, &env, false)
        .await
        .expect("the cancel extras are accepted and dropped");
    let cancel_requests = cancel_wire.requests();
    assert_eq!(cancel_requests.len(), 1);
    assert_eq!(body_json(&cancel_requests[0]), json("{}"));
    assert_eq!(
        header(&cancel_requests[0], "x-trace").as_deref(),
        Some("kept")
    );
    drop(cancel_requests);

    let rows = [
        (None, Duration::from_secs(6000)),
        (Some(0.0_f64), Duration::from_secs(6000)),
        (Some(7.0_f64), Duration::from_secs(7)),
        (Some(-3.0_f64), Duration::ZERO),
    ];
    let timed_wire = Wire::returning(200, DELETED);
    let timed_source = Source::new(timed_wire.clone());
    let timed_service = writing(&config, &timed_source, configured(&[]));
    for (timeout, expected) in rows {
        let mut timed_call = configured_call(LifecycleOperation::Delete, "resp_a");
        timed_call.custom_llm_provider = Some(String::from("openai"));
        timed_call.timeout = timeout;
        deleted(&timed_service, &timed_call, &mut state, &env, false)
            .await
            .expect("the timeout is forwarded");
        let sent = timed_wire.requests();
        assert_eq!(sent[sent.len() - 1].timeout, Some(expected));
        drop(sent);
    }
    let injected_wire = Wire::returning(200, DELETED);
    let injected_source = Source::new(injected_wire.clone());
    let injected = writing(
        &config,
        &injected_source,
        configured(&[("REQUEST_TIMEOUT", "17")]),
    );
    for timeout in [None, Some(0.0_f64)] {
        let mut injected_call = configured_call(LifecycleOperation::Delete, "resp_a");
        injected_call.custom_llm_provider = Some(String::from("openai"));
        injected_call.timeout = timeout;
        deleted(&injected, &injected_call, &mut state, &env, false)
            .await
            .expect("the injected request timeout is the fallback");
        let sent = injected_wire.requests();
        assert_eq!(sent[sent.len() - 1].timeout, Some(Duration::from_secs(17)));
        drop(sent);
    }
}

#[tokio::test]
async fn lifecycle_write_rejections() {
    let settings = credentials();
    let config = openai_config(&settings);
    let harness = Harness::new();
    let env = harness.pre_post();
    let missing = [
        (
            LifecycleOperation::Delete,
            "custom_llm_provider is required but passed as None",
        ),
        (
            LifecycleOperation::Cancel,
            "custom_llm_provider is required but passed as None",
        ),
    ];
    for (operation, message) in missing {
        let wire = Wire::returning(200, DELETED);
        let source = Source::new(wire.clone());
        let service = writing(&config, &source, configured(&[]));
        let mut state = logging_state("gpt-5", "call-1");
        let call = configured_call(operation, "resp_a");
        let result = match operation {
            LifecycleOperation::Delete => deleted(&service, &call, &mut state, &env, false)
                .await
                .map(drop),
            LifecycleOperation::Cancel => cancelled(&service, &call, &mut state, &env, false)
                .await
                .map(drop),
        };
        let error = result.expect_err("a missing provider is refused");
        assert!(matches!(&error.error, LifecycleWriteError::MissingProvider));
        assert_eq!(error.error.to_string(), message);
        assert_api_connection(&harness.class(&error));
        assert!(wire.requests().is_empty());
    }
    let unsupported = [
        (
            LifecycleOperation::Delete,
            "DELETE responses is not supported for anthropic",
        ),
        (
            LifecycleOperation::Cancel,
            "CANCEL responses is not supported for anthropic",
        ),
    ];
    for (operation, message) in unsupported {
        let wire = Wire::returning(200, DELETED);
        let source = Source::new(wire.clone());
        let service = writing(&config, &source, configured(&[]));
        let mut state = logging_state("gpt-5", "call-1");
        let mut call = configured_call(operation, "resp_a");
        call.custom_llm_provider = Some(String::from("anthropic"));
        let result = match operation {
            LifecycleOperation::Delete => deleted(&service, &call, &mut state, &env, false)
                .await
                .map(drop),
            LifecycleOperation::Cancel => cancelled(&service, &call, &mut state, &env, false)
                .await
                .map(drop),
        };
        let error = result.expect_err("an unsupported provider is refused");
        assert!(matches!(
            &error.error,
            LifecycleWriteError::Unsupported { provider, .. } if provider.as_str() == "anthropic"
        ));
        assert_eq!(error.error.to_string(), message);
        assert_api_connection(&harness.class(&error));
        assert!(wire.requests().is_empty());
        assert_eq!(state.optional_params.len(), 0);
        assert_eq!(state.litellm_params.len(), 0);
    }
    let wire = Wire::returning(
        404,
        r#"{"error":{"message":"gone","type":"invalid_request_error","param":"response_id","code":"not_found"}}"#,
    );
    let source = Source::new(wire.clone());
    let service = writing(&config, &source, configured(&[]));
    let mut state = logging_state("gpt-5", "call-1");
    let mut call = configured_call(LifecycleOperation::Delete, "resp_a");
    call.custom_llm_provider = Some(String::from("openai"));
    let error = deleted(&service, &call, &mut state, &env, false)
        .await
        .expect_err("a 404 is a provider failure");
    let LifecycleWriteError::Transport(WriteError::Provider(provider_error)) = &error.error else {
        panic!("the provider's own exception reaches the boundary");
    };
    assert_eq!(provider_error.status_code, 404);
    let class = harness.class(&error);
    let ErrorClass::NotFound(_) = &class else {
        panic!("the 404 maps to the SDK's NotFoundError");
    };
    let metadata = class.metadata();
    assert_eq!(metadata.status_code, 404);
    assert_eq!(metadata.error_type, WireField::Null);
    assert_eq!(metadata.param, WireField::Null);
    assert_eq!(metadata.openai_code, None);
    assert_eq!(wire.requests().len(), 1);
}

#[tokio::test]
async fn lifecycle_write_context() {
    let settings = credentials();
    let config = openai_config(&settings);
    let harness = Harness::new();
    let supplied = Wire::returning(200, DELETED);
    let created = Wire::returning(200, DELETED);
    let source = Source::new(created.clone());
    let deps = WriteDeps {
        config: &config,
        source: &source,
        supplied: Some(SuppliedClient {
            kind: ClientKind::Sync,
            transport: &supplied,
        }),
        shared_session: None,
    };
    let service = ResponsesLifecycleWrite {
        deps,
        request_timeout: configured(&[]),
    };
    let rows = [
        (None, false, true),
        (Some(Json::Number(Number::from(1_u64))), false, true),
        (Some(Json::Text(String::from("true"))), false, true),
        (Some(Json::Bool(true)), false, false),
        (None, true, false),
    ];
    let mut supplied_calls: usize = 0;
    let mut created_calls: usize = 0;
    for (flag, asynchronous, supplied_used) in rows {
        let mut call = configured_call(LifecycleOperation::Delete, "resp_a");
        call.custom_llm_provider = Some(String::from("openai"));
        call.adelete_responses = flag;
        call.params.litellm_call_id = Some(String::from("call-a"));
        let mut state = logging_state("gpt-5", "call-1");
        let env = harness.pre_post();
        let answer = deleted(&service, &call, &mut state, &env, asynchronous)
            .await
            .expect("the flag selects the client the sync entry sends with");
        assert_eq!(answer.id.as_deref(), Some("resp_a"));
        assert_eq!(
            state.litellm_params.get("litellm_call_id"),
            Some(&Json::Text(String::from("call-a")))
        );
        assert_eq!(state.litellm_params.len(), 1);
        assert_eq!(
            state.optional_params.get("response_id"),
            Some(&Json::Text(String::from("resp_a")))
        );
        assert_eq!(state.optional_params.len(), 1);
        assert_eq!(state.model, "gpt-5");
        if supplied_used {
            supplied_calls += 1;
        } else {
            created_calls += 1;
        }
        assert_eq!(supplied.requests().len(), supplied_calls);
        assert_eq!(created.requests().len(), created_calls);
    }
    let rejected_wire = Wire::returning(200, DELETED);
    let rejected_source = Source::new(rejected_wire.clone());
    let rejected_service = writing(&config, &rejected_source, configured(&[]));
    let mut rejected_state = logging_state("gpt-5", "call-1");
    let env = harness.pre_post();
    let mut rejected_call = configured_call(LifecycleOperation::Delete, "resp_a");
    rejected_call.params.litellm_call_id = Some(String::from("call-a"));
    rejected_service
        .delete_responses(&rejected_call, &mut rejected_state, &env)
        .await
        .expect_err("the provider is missing");
    assert_eq!(rejected_state.optional_params.len(), 0);
    assert_eq!(rejected_state.litellm_params.len(), 0);
    assert!(rejected_wire.requests().is_empty());
    let cancel_wire = Wire::returning(200, CANCELLED);
    let cancel_source = Source::new(cancel_wire.clone());
    let cancel_service = writing(&config, &cancel_source, configured(&[]));
    let mut cancel_state = logging_state("gpt-5", "call-1");
    let mut cancel_call = configured_call(LifecycleOperation::Cancel, "resp_a");
    cancel_call.custom_llm_provider = Some(String::from("openai"));
    cancelled(&cancel_service, &cancel_call, &mut cancel_state, &env, true)
        .await
        .expect("the async cancel answers once");
    assert_eq!(cancel_wire.requests().len(), 1);
    assert_eq!(cancel_wire.requests()[0].method, Method::POST);
}
