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
use betterlite_openai::response_resources::error::ResponseResourceError;
use betterlite_openai::response_resources::request::{ResourceOperation, resource_request};
use betterlite_openai::response_resources::response::TypedReply;
use http::Method;
use rstest::rstest;
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

fn injected_timeout(injected: Option<&str>) -> RequestTimeout {
    injected.map_or_else(
        || configured(&[]),
        |seconds| configured(&[("REQUEST_TIMEOUT", seconds)]),
    )
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

// The answer either entry point gives, boxed so the two variants keep one size; one helper drives both operations in both modes
enum Answer {
    Deleted(Box<DeleteResponseResult>),
    Cancelled(Box<TypedReply>),
}

impl Answer {
    fn deleted(answer: DeleteResponseResult) -> Self {
        Self::Deleted(Box::new(answer))
    }

    fn cancelled(answer: TypedReply) -> Self {
        Self::Cancelled(Box::new(answer))
    }

    fn into_deleted(self) -> DeleteResponseResult {
        match self {
            Self::Deleted(answer) => *answer,
            Self::Cancelled(_) => panic!("a delete call answered with the cancel result"),
        }
    }

    fn into_cancelled(self) -> TypedReply {
        match self {
            Self::Cancelled(answer) => *answer,
            Self::Deleted(_) => panic!("a cancel call answered with the delete result"),
        }
    }
}

async fn answered(
    service: &ResponsesLifecycleWrite<'_, OpenAiResponsesConfig<'_>, Source>,
    call: &ResponsesLifecycleWriteCall,
    state: &mut LoggingState,
    env: &PrePostEnv<'_>,
    asynchronous: bool,
) -> Result<Answer, ResponsesLifecycleWriteError> {
    match (call.operation, asynchronous) {
        (LifecycleOperation::Delete, false) => service
            .delete_responses(call, state, env)
            .await
            .map(Answer::deleted),
        (LifecycleOperation::Delete, true) => service
            .adelete_responses(call, state, env)
            .await
            .map(Answer::deleted),
        (LifecycleOperation::Cancel, false) => service
            .cancel_responses(call, state, env)
            .await
            .map(Answer::cancelled),
        (LifecycleOperation::Cancel, true) => service
            .acancel_responses(call, state, env)
            .await
            .map(Answer::cancelled),
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

fn assert_wire(request: &WriteRequest, method: &Method, path: &str) {
    assert_eq!(&request.method, method);
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

#[rstest]
#[case::sync(false)]
#[case::asynchronous(true)]
#[tokio::test]
async fn delete_and_cancel_encoded_provider(#[case] asynchronous: bool) {
    let settings = credentials();
    let config = openai_config(&settings);
    let harness = Harness::new();
    let encoded = build_response_id(Some("openai"), Some("model-a"), "resp/a");
    let metadata = json(r#"{"model_info":{"id":"deployment_a"}}"#);
    let wire = Wire::returning(200, DELETED);
    let source = Source::new(wire.clone());
    let service = writing(&config, &source, configured(&[]));
    let mut state = logging_state("gpt-5", "call-1");
    let env = harness.pre_post();
    let mut prepared = configured_call(LifecycleOperation::Delete, &encoded);
    prepared.custom_llm_provider = Some(String::from("anthropic"));
    prepared.params.litellm_metadata = Some(metadata.clone());
    let answer = answered(&service, &prepared, &mut state, &env, asynchronous)
        .await
        .expect("the encoded provider wins over the explicit one")
        .into_deleted();
    assert_eq!(answer.id.as_deref(), Some("resp_a"));
    assert_eq!(answer.object.as_deref(), Some("response"));
    assert_eq!(answer.deleted, Some(true));
    assert_eq!(state.litellm_params.get("metadata"), Some(&metadata));
    assert_eq!(
        state.litellm_params.get("litellm_metadata"),
        Some(&metadata)
    );
    assert_eq!(wire.requests().len(), 1);
    assert_wire(&wire.requests()[0], &Method::DELETE, "resp%2Fa");
    assert!(matches!(&wire.requests()[0].body, RequestBody::Empty));

    let cancel_wire = Wire::returning(200, CANCELLED);
    let cancel_source = Source::new(cancel_wire.clone());
    let cancel_service = writing(&config, &cancel_source, configured(&[]));
    let mut cancel_state = logging_state("gpt-5", "call-2");
    let mut cancel_call = configured_call(LifecycleOperation::Cancel, &encoded);
    cancel_call.custom_llm_provider = Some(String::from("anthropic"));
    cancel_call.params.litellm_metadata = Some(metadata);
    let reply = answered(
        &cancel_service,
        &cancel_call,
        &mut cancel_state,
        &env,
        asynchronous,
    )
    .await
    .expect("the encoded provider wins for the cancel too")
    .into_cancelled();
    assert_eq!(reply.response.id, "resp_a");
    assert_eq!(reply.response.status.as_deref(), Some("cancelled"));
    assert_eq!(reply.response.model.as_deref(), Some("gpt-5"));
    assert_eq!(cancel_wire.requests().len(), 1);
    assert_wire(&cancel_wire.requests()[0], &Method::POST, "resp%2Fa/cancel");
    assert_eq!(body_json(&cancel_wire.requests()[0]), json("{}"));

    let plain_wire = Wire::returning(200, DELETED);
    let plain_source = Source::new(plain_wire.clone());
    let plain_service = writing(&config, &plain_source, configured(&[]));
    let mut plain_state = logging_state("gpt-5", "call-3");
    let plain_env = harness.pre_post();
    let mut plain_call = configured_call(LifecycleOperation::Delete, "resp_a");
    plain_call.custom_llm_provider = Some(String::from("openai"));
    answered(
        &plain_service,
        &plain_call,
        &mut plain_state,
        &plain_env,
        false,
    )
    .await
    .expect("a raw id with an explicit openai provider succeeds")
    .into_deleted();
    assert_wire(&plain_wire.requests()[0], &Method::DELETE, "resp_a");
    let empty_provider = build_response_id(Some(""), Some("model-a"), "resp_b");
    let mut provider_call = configured_call(LifecycleOperation::Delete, &empty_provider);
    provider_call.custom_llm_provider = Some(String::from("openai"));
    answered(
        &plain_service,
        &provider_call,
        &mut plain_state,
        &plain_env,
        false,
    )
    .await
    .expect("an empty decoded provider falls back to the argument")
    .into_deleted();
    assert_wire(&plain_wire.requests()[1], &Method::DELETE, "resp_b");
    let empty_id = build_response_id(Some("openai"), Some("model-a"), "");
    let mut id_call = configured_call(LifecycleOperation::Delete, &empty_id);
    id_call.custom_llm_provider = Some(String::from("openai"));
    answered(
        &plain_service,
        &id_call,
        &mut plain_state,
        &plain_env,
        false,
    )
    .await
    .expect("an empty decoded id falls back to the argument")
    .into_deleted();
    let expected = resource_request(
        &empty_id,
        &format!("{BASE}/responses"),
        ResourceOperation::Delete,
    )
    .expect("the fallback id encodes");
    assert_eq!(plain_wire.requests()[2].url.as_str(), expected.url);
}

#[rstest]
#[case::unset(None, None, Duration::from_mins(100))]
#[case::zero(None, Some(0.0_f64), Duration::from_mins(100))]
#[case::seven(None, Some(7.0_f64), Duration::from_secs(7))]
#[case::negative(None, Some(-3.0_f64), Duration::ZERO)]
#[case::injected_unset(Some("17"), None, Duration::from_secs(17))]
#[case::injected_zero(Some("17"), Some(0.0_f64), Duration::from_secs(17))]
#[tokio::test]
async fn lifecycle_write_ignored_extras(
    #[case] injected: Option<&str>,
    #[case] timeout: Option<f64>,
    #[case] expected: Duration,
) {
    let settings = credentials();
    let config = openai_config(&settings);
    let harness = Harness::new();
    let trace: BTreeMap<String, String> =
        BTreeMap::from([("X-Trace".to_owned(), "kept".to_owned())]);
    let mut delete_call = configured_call(LifecycleOperation::Delete, "resp_a");
    delete_call.custom_llm_provider = Some(String::from("openai"));
    delete_call.extra_headers = Some(trace.clone());
    delete_call.extra_query = Some(extra(r#"{"trace":"ignored"}"#));
    delete_call.extra_body = Some(extra(r#"{"reason":"ignored"}"#));
    delete_call.timeout = timeout;
    let wire = Wire::returning(200, DELETED);
    let source = Source::new(wire.clone());
    let service = writing(&config, &source, injected_timeout(injected));
    let mut state = logging_state("gpt-5", "call-1");
    let env = harness.pre_post();
    answered(&service, &delete_call, &mut state, &env, false)
        .await
        .expect("the extras are accepted and dropped")
        .into_deleted();
    assert_eq!(wire.requests().len(), 1);
    assert_wire(&wire.requests()[0], &Method::DELETE, "resp_a");
    assert_eq!(
        header(&wire.requests()[0], "x-trace").as_deref(),
        Some("kept")
    );
    assert!(matches!(&wire.requests()[0].body, RequestBody::Empty));
    assert_eq!(wire.requests()[0].timeout, Some(expected));

    let cancel_wire = Wire::returning(200, CANCELLED);
    let cancel_source = Source::new(cancel_wire.clone());
    let cancel_service = writing(&config, &cancel_source, injected_timeout(injected));
    let mut cancel_call = configured_call(LifecycleOperation::Cancel, "resp_a");
    cancel_call.custom_llm_provider = Some(String::from("openai"));
    cancel_call.extra_headers = Some(trace);
    cancel_call.extra_query = Some(extra(r#"{"trace":"ignored"}"#));
    cancel_call.extra_body = Some(extra(r#"{"reason":"ignored"}"#));
    cancel_call.timeout = timeout;
    answered(&cancel_service, &cancel_call, &mut state, &env, false)
        .await
        .expect("the cancel extras are accepted and dropped")
        .into_cancelled();
    assert_eq!(cancel_wire.requests().len(), 1);
    assert_wire(&cancel_wire.requests()[0], &Method::POST, "resp_a/cancel");
    assert_eq!(body_json(&cancel_wire.requests()[0]), json("{}"));
    assert_eq!(
        header(&cancel_wire.requests()[0], "x-trace").as_deref(),
        Some("kept")
    );
    assert_eq!(cancel_wire.requests()[0].timeout, Some(expected));
}

#[rstest]
#[case::delete(LifecycleOperation::Delete)]
#[case::cancel(LifecycleOperation::Cancel)]
#[tokio::test]
async fn lifecycle_write_rejections(#[case] operation: LifecycleOperation) {
    let settings = credentials();
    let config = openai_config(&settings);
    let harness = Harness::new();
    let env = harness.pre_post();
    let wire = Wire::returning(200, DELETED);
    let source = Source::new(wire.clone());
    let service = writing(&config, &source, configured(&[]));
    let mut state = logging_state("gpt-5", "call-1");
    let call = configured_call(operation, "resp_a");
    let error = answered(&service, &call, &mut state, &env, false)
        .await
        .map(drop)
        .expect_err("a missing provider is refused");
    assert!(matches!(&error.error, LifecycleWriteError::MissingProvider));
    assert_eq!(
        error.error.to_string(),
        "custom_llm_provider is required but passed as None"
    );
    assert_api_connection(&harness.class(&error));
    assert!(wire.requests().is_empty());

    let mut unsupported_call = configured_call(operation, "resp_a");
    unsupported_call.custom_llm_provider = Some(String::from("anthropic"));
    let unsupported_error = answered(&service, &unsupported_call, &mut state, &env, false)
        .await
        .map(drop)
        .expect_err("an unsupported provider is refused");
    assert!(matches!(
        &unsupported_error.error,
        LifecycleWriteError::Unsupported { provider, .. } if provider.as_str() == "anthropic"
    ));
    let refusal = match operation {
        LifecycleOperation::Delete => "DELETE responses is not supported for anthropic",
        LifecycleOperation::Cancel => "CANCEL responses is not supported for anthropic",
    };
    assert_eq!(unsupported_error.error.to_string(), refusal);
    assert_api_connection(&harness.class(&unsupported_error));
    assert!(wire.requests().is_empty());
    assert_eq!(state.optional_params.len(), 0);
    assert_eq!(state.litellm_params.len(), 0);

    let not_found_wire = Wire::returning(
        404,
        r#"{"error":{"message":"gone","type":"invalid_request_error","param":"response_id","code":"not_found"}}"#,
    );
    let not_found_source = Source::new(not_found_wire.clone());
    let not_found_service = writing(&config, &not_found_source, configured(&[]));
    let mut not_found_state = logging_state("gpt-5", "call-1");
    let mut not_found_call = configured_call(LifecycleOperation::Delete, "resp_a");
    not_found_call.custom_llm_provider = Some(String::from("openai"));
    let not_found_error = answered(
        &not_found_service,
        &not_found_call,
        &mut not_found_state,
        &env,
        false,
    )
    .await
    .map(drop)
    .expect_err("a 404 is a provider failure");
    let LifecycleWriteError::Transport(WriteError::Provider(provider_error)) =
        &not_found_error.error
    else {
        panic!("the provider's own exception reaches the boundary");
    };
    assert_eq!(provider_error.status_code, 404);
    let class = harness.class(&not_found_error);
    let ErrorClass::BadRequest(_) = &class else {
        panic!("the invalid_request_error body maps to the SDK's BadRequestError");
    };
    let metadata = class.metadata();
    assert_eq!(metadata.status_code, 404);
    assert_eq!(metadata.error_type, WireField::Null);
    assert_eq!(metadata.param, WireField::Null);
    assert_eq!(metadata.openai_code, None);
    assert_eq!(not_found_wire.requests().len(), 1);

    let unparsed_wire = Wire::returning(200, "not json");
    let unparsed_source = Source::new(unparsed_wire.clone());
    let unparsed_service = writing(&config, &unparsed_source, configured(&[]));
    let mut unparsed_state = logging_state("gpt-5", "call-1");
    let mut unparsed_call = configured_call(operation, "resp_a");
    unparsed_call.custom_llm_provider = Some(String::from("openai"));
    let unparsed_error = answered(
        &unparsed_service,
        &unparsed_call,
        &mut unparsed_state,
        &env,
        false,
    )
    .await
    .map(drop)
    .expect_err("a success reply that is not JSON fails the transform");
    let LifecycleWriteError::Transport(WriteError::Resource(error)) = &unparsed_error.error else {
        panic!("the transform's own exception reaches the boundary");
    };
    assert_eq!(error.http_status(), Some(200));
    let unparsed_class = harness.class(&unparsed_error);
    let ErrorClass::Api(_) = &unparsed_class else {
        panic!("the transform's OpenAIError becomes the SDK's APIError");
    };
    let unparsed_metadata = unparsed_class.metadata();
    assert_eq!(unparsed_metadata.status_code, 200);
    assert_eq!(unparsed_metadata.error_type, WireField::Null);
    assert_eq!(unparsed_metadata.param, WireField::Null);
    assert_eq!(unparsed_metadata.openai_code, None);
    assert_eq!(unparsed_wire.requests().len(), 1);

    let record_wire = Wire::returning(200, "[]");
    let record_source = Source::new(record_wire.clone());
    let record_service = writing(&config, &record_source, configured(&[]));
    let mut record_state = logging_state("gpt-5", "call-1");
    let record_error = answered(
        &record_service,
        &unparsed_call,
        &mut record_state,
        &env,
        false,
    )
    .await
    .map(drop)
    .expect_err("a decoded reply that does not match the record fails the transform");
    let LifecycleWriteError::Transport(WriteError::Resource(error)) = &record_error.error else {
        panic!("the transform's own exception reaches the boundary");
    };
    assert!(matches!(error, ResponseResourceError::Record { .. }));
    assert_eq!(error.http_status(), None);
    assert_api_connection(&harness.class(&record_error));
    assert_eq!(record_wire.requests().len(), 1);
}

#[rstest]
#[case::unset(None, false, true)]
#[case::number(Some(Json::Number(Number::from(1_u64))), false, true)]
#[case::text(Some(Json::Text(String::from("true"))), false, true)]
#[case::flag_true(Some(Json::Bool(true)), false, false)]
#[case::async_entry(None, true, false)]
#[tokio::test]
async fn lifecycle_write_context(
    #[case] flag: Option<Json>,
    #[case] asynchronous: bool,
    #[case] supplied_used: bool,
) {
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
    let mut call = configured_call(LifecycleOperation::Delete, "resp_a");
    call.custom_llm_provider = Some(String::from("openai"));
    call.adelete_responses = flag;
    call.params.litellm_call_id = Some(String::from("call-a"));
    let mut state = logging_state("gpt-5", "call-1");
    let env = harness.pre_post();
    let answer = answered(&service, &call, &mut state, &env, asynchronous)
        .await
        .expect("the flag selects the client the sync entry sends with")
        .into_deleted();
    assert_eq!(answer.id.as_deref(), Some("resp_a"));
    assert_eq!(
        state.litellm_params.get("litellm_call_id"),
        Some(&Json::Text(String::from("call-a")))
    );
    assert_eq!(
        state.litellm_params.get("api_base"),
        Some(&Json::Text(format!("{BASE}/responses")))
    );
    assert_eq!(state.litellm_params.len(), 2);
    assert_eq!(
        state.optional_params.get("response_id"),
        Some(&Json::Text(String::from("resp_a")))
    );
    assert_eq!(state.optional_params.len(), 1);
    assert_eq!(state.model, "gpt-5");
    if supplied_used {
        assert_eq!(supplied.requests().len(), 1);
        assert!(created.requests().is_empty());
    } else {
        assert_eq!(created.requests().len(), 1);
        assert!(supplied.requests().is_empty());
    }

    let rejected_wire = Wire::returning(200, DELETED);
    let rejected_source = Source::new(rejected_wire.clone());
    let rejected_service = writing(&config, &rejected_source, configured(&[]));
    let mut rejected_state = logging_state("gpt-5", "call-1");
    let rejected_env = harness.pre_post();
    let mut rejected_call = configured_call(LifecycleOperation::Delete, "resp_a");
    rejected_call.params.litellm_call_id = Some(String::from("call-a"));
    rejected_service
        .delete_responses(&rejected_call, &mut rejected_state, &rejected_env)
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
    answered(&cancel_service, &cancel_call, &mut cancel_state, &env, true)
        .await
        .expect("the async cancel answers once")
        .into_cancelled();
    assert_eq!(cancel_wire.requests().len(), 1);
    assert_eq!(cancel_wire.requests()[0].method, Method::POST);
}
