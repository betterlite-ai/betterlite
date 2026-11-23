#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_error::error::ErrorClass;
use betterlite_http::proxy::pool::PoolError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::api::container_contracts::error::{BaseLlmException, ContainerError};
use betterlite_openai::containers::container_file_http::ContainerFileHttpError;
use indexmap::IndexMap;
use serde_json::Number;

use super::super::error::ContainerOpFailure;
use super::support::{
    API_BASE, API_KEY, API_VERSION, CALL_ID, CONTAINER_ID, Fixture, HandlerForm, INITIAL_CALL_TYPE,
    StubLookup, create_call, declared_timeout, full_create_call, headers, object, query,
    request_timeout, routing_metadata, secret,
};
use crate::api::container_collection_http::ContainerCollectionError;
use crate::api::response_id_codecs::container::build_container_id;

fn provider_error(status_code: u16, message: &str) -> BaseLlmException {
    BaseLlmException {
        status_code,
        message: message.to_owned(),
        headers: BTreeMap::new(),
    }
}

fn handler_failure(error: ContainerCollectionError) -> ContainerOpFailure {
    let mut fixture = Fixture::new();
    fixture.handler.fail_with(error);
    fixture
        .create(&create_call())
        .expect_err("the handler failed the call")
}

#[test]
fn create_hands_the_name_and_mapped_params_to_the_handler() {
    let fixture = Fixture::new();

    fixture
        .create(&full_create_call())
        .expect("the handler answers");

    let creates = fixture.handler.creates();
    assert_eq!(creates.len(), 1);
    assert_eq!(creates[0].base.form, HandlerForm::Blocking);
    assert_eq!(creates[0].name, "store");
    assert_eq!(creates[0].base.headers, Some(headers()));
    assert_eq!(
        creates[0].params,
        object(
            r#"{"expires_after":{"anchor":"last_active_at","minutes":30},"file_ids":["file-1"],"extra_headers":{"X-Test":"1"}}"#
        )
    );
}

#[test]
fn create_logs_the_call_type_and_the_mapped_params() {
    let fixture = Fixture::new();

    fixture
        .create(&full_create_call())
        .expect("the handler answers");

    let state = fixture.state();
    assert_eq!(state.call_type, "create_container");
    assert_eq!(state.model, "");
    assert_eq!(state.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(
        state.litellm_params.get("litellm_call_id"),
        Some(&Json::Text(CALL_ID.to_owned()))
    );
    assert_eq!(
        state.litellm_params.get("file_ids"),
        Some(&Json::List(vec![Json::Text("file-1".to_owned())]))
    );
    assert!(state.litellm_params.get("api_key").is_none());
}

#[test]
fn create_logs_the_mapped_optional_params() {
    let fixture = Fixture::new();

    fixture
        .create(&full_create_call())
        .expect("the handler answers");

    let state = fixture.state();
    assert_eq!(
        state.optional_params.get("expires_after"),
        Some(&Json::Object(IndexMap::from([
            ("anchor".to_owned(), Json::Text("last_active_at".to_owned())),
            ("minutes".to_owned(), Json::Number(Number::from(30_i64))),
        ])))
    );
    assert_eq!(
        state.optional_params.get("file_ids"),
        Some(&Json::List(vec![Json::Text("file-1".to_owned())]))
    );
    assert_eq!(
        state.optional_params.get("extra_headers"),
        Some(&Json::Object(IndexMap::from([(
            "X-Test".to_owned(),
            Json::Text("1".to_owned()),
        )])))
    );
    // the unsupported extra_body and the separately named name never enter the logged params
    assert!(state.optional_params.get("extra_body").is_none());
    assert!(state.optional_params.get("name").is_none());
}

#[test]
fn create_the_handler_sees_the_logging_state_with_the_call_type_set() {
    let fixture = Fixture::new();
    let mut call = create_call();
    call.name = "store".to_owned();

    fixture.create(&call).expect("the handler answers");

    let creates = fixture.handler.creates();
    assert_eq!(creates[0].base.call_type, "create_container");
    assert_eq!(creates[0].base.provider.as_deref(), Some("openai"));
    assert_eq!(creates[0].base.model, "");
    assert!(creates[0].base.optional_params.is_empty());
}

#[test]
fn create_uses_the_default_call_timeout_of_six_hundred_seconds() {
    let fixture = Fixture::new();
    let call = create_call();

    fixture.create(&call).expect("the handler answers");

    let creates = fixture.handler.creates();
    assert_eq!(creates[0].base.timeout, Some(declared_timeout()));
}

#[test]
fn create_an_absent_timeout_falls_back_to_the_request_timeout() {
    let fixture = Fixture::new();
    let mut call = create_call();
    call.timeout = None;

    fixture.create(&call).expect("the handler answers");

    let creates = fixture.handler.creates();
    assert_eq!(creates[0].base.timeout, Some(request_timeout()));
}

#[test]
fn create_a_declared_timeout_reaches_the_handler() {
    let fixture = Fixture::new();
    let mut call = create_call();
    call.timeout = Some(5.0_f64);

    fixture.create(&call).expect("the handler answers");

    let creates = fixture.handler.creates();
    assert_eq!(creates[0].base.timeout, Some(Duration::from_secs(5)));
}

#[test]
fn create_a_zero_timeout_falls_back_to_the_request_timeout() {
    let fixture = Fixture::new();
    let mut call = create_call();
    call.timeout = Some(0.0_f64);

    fixture.create(&call).expect("the handler answers");

    let creates = fixture.handler.creates();
    assert_eq!(creates[0].base.timeout, Some(request_timeout()));
}

#[test]
fn create_a_negative_timeout_is_a_value_error() {
    let fixture = Fixture::new();
    let mut call = create_call();
    call.timeout = Some(-1.0_f64);

    let failure = fixture.create(&call).expect_err("the timeout is unusable");

    let ErrorClass::ApiConnection(context) = &failure.class else {
        panic!("expected an api connection, got {:?}", failure.class);
    };
    assert_eq!(context.model.as_deref(), Some(""));
    assert_eq!(context.provider.as_deref(), Some("openai"));
    assert!(context.message.contains("is not a usable duration"));
    assert!(fixture.handler.creates().is_empty());
}

#[test]
fn create_defaults_to_the_openai_provider() {
    let fixture = Fixture::new();
    let call = create_call();

    fixture.create(&call).expect("the handler answers");

    assert_eq!(call.custom_llm_provider, "openai");
    assert_eq!(fixture.lookup.asked(), vec![ProviderId::Openai]);
    assert_eq!(
        fixture.state().custom_llm_provider.as_deref(),
        Some("openai")
    );
}

#[test]
fn create_azure_text_resolves_to_the_azure_config() {
    let fixture = Fixture::new();
    let mut call = create_call();
    call.custom_llm_provider = "azure_text".to_owned();

    fixture.create(&call).expect("the handler answers");

    assert_eq!(fixture.lookup.asked(), vec![ProviderId::Azure]);
    let state = fixture.state();
    assert_eq!(state.custom_llm_provider.as_deref(), Some("azure_text"));
}

#[test]
fn create_a_known_provider_without_a_container_config_is_unsupported() {
    let fixture = Fixture::new();
    let mut call = create_call();
    call.custom_llm_provider = "anthropic".to_owned();

    let failure = fixture
        .create(&call)
        .expect_err("no container config answers anthropic");

    assert_eq!(
        failure.error.to_string(),
        "container operations are not supported for anthropic"
    );
    let ErrorClass::BadRequest(details) = &failure.class else {
        panic!("expected a bad request, got {:?}", failure.class);
    };
    assert_eq!(details.status.0, 400);
    assert_eq!(
        details.message,
        "container operations are not supported for anthropic"
    );
    assert_eq!(details.provider.as_deref(), Some("anthropic"));
    assert_eq!(details.model.as_deref(), Some(""));
    assert!(fixture.lookup.asked().is_empty());
    assert!(fixture.handler.creates().is_empty());
    assert_eq!(fixture.state().call_type, INITIAL_CALL_TYPE);
}

#[test]
fn create_a_provider_betterlite_does_not_know_is_a_bad_request() {
    let fixture = Fixture::new();
    let mut call = create_call();
    call.custom_llm_provider = "acme".to_owned();

    let failure = fixture.create(&call).expect_err("acme is no provider");

    assert!(failure.error.to_string().contains("acme"));
    let ErrorClass::BadRequest(details) = &failure.class else {
        panic!("expected a bad request, got {:?}", failure.class);
    };
    assert_eq!(details.status.0, 400);
    assert_eq!(details.provider.as_deref(), Some("acme"));
    assert_eq!(details.model.as_deref(), Some(""));
    assert!(fixture.lookup.asked().is_empty());
    assert!(fixture.handler.creates().is_empty());
}

#[test]
fn create_a_provider_config_the_registry_lacks_is_unsupported() {
    let mut fixture = Fixture::new();
    fixture.lookup = StubLookup::empty();

    let failure = fixture
        .create(&create_call())
        .expect_err("the registry answers no config");

    assert_eq!(
        failure.error.to_string(),
        "container operations are not supported for openai"
    );
    let ErrorClass::BadRequest(details) = &failure.class else {
        panic!("expected a bad request, got {:?}", failure.class);
    };
    assert_eq!(details.status.0, 400);
    assert_eq!(
        details.message,
        "container operations are not supported for openai"
    );
    assert_eq!(details.provider.as_deref(), Some("openai"));
    assert_eq!(details.model.as_deref(), Some(""));
    assert_eq!(fixture.lookup.asked(), vec![ProviderId::Openai]);
    assert!(fixture.handler.creates().is_empty());
}

#[test]
fn create_passes_credentials_and_extra_kwargs_to_the_handler() {
    let fixture = Fixture::new();
    let mut call = create_call();
    call.api_key = Some(secret(API_KEY));
    call.api_base = Some(API_BASE.to_owned());
    call.api_version = Some(API_VERSION.to_owned());
    call.litellm_call_id = Some(CALL_ID.to_owned());
    call.litellm_metadata = Some(routing_metadata("gpt-4o"));
    call.extra.insert("ssl_verify".to_owned(), Json::Bool(true));

    fixture.create(&call).expect("the handler answers");

    let creates = fixture.handler.creates();
    assert_eq!(
        creates[0].base.litellm_params,
        object(concat!(
            r#"{"api_base":"https://api.openai.com/v1","api_key":"sk-test","#,
            r#""api_version":"2024-06-01","litellm_call_id":"call-1","#,
            r#""litellm_metadata":{"model_info":{"id":"gpt-4o"}},"ssl_verify":true}"#,
        ))
    );
}

#[test]
fn create_the_named_credentials_win_over_the_same_kwargs() {
    let fixture = Fixture::new();
    let mut call = create_call();
    call.api_key = Some(secret(API_KEY));
    call.api_base = Some(API_BASE.to_owned());
    call.extra
        .insert("api_key".to_owned(), Json::Text("sk-kwarg".to_owned()));
    call.extra.insert(
        "api_base".to_owned(),
        Json::Text("https://kwarg.example".to_owned()),
    );

    fixture.create(&call).expect("the handler answers");

    let creates = fixture.handler.creates();
    assert_eq!(
        creates[0]
            .base
            .litellm_params
            .get("api_key")
            .and_then(|value| value.as_str()),
        Some(API_KEY)
    );
    assert_eq!(
        creates[0]
            .base
            .litellm_params
            .get("api_base")
            .and_then(|value| value.as_str()),
        Some(API_BASE)
    );
}

#[test]
fn create_removes_the_logging_object_and_async_flag_from_the_kwargs() {
    let fixture = Fixture::new();
    let mut call = create_call();
    call.extra.insert("async_call".to_owned(), Json::Bool(true));
    call.extra.insert(
        "litellm_logging_obj".to_owned(),
        Json::Text("object".to_owned()),
    );
    call.extra
        .insert("custom".to_owned(), Json::Number(Number::from(1_i64)));

    fixture.create(&call).expect("the handler answers");

    let creates = fixture.handler.creates();
    let params = &creates[0].base.litellm_params;
    assert!(params.get("async_call").is_none());
    assert!(params.get("litellm_logging_obj").is_none());
    assert_eq!(params.get("custom"), Some(&serde_json::json!(1_i64)));
    let state = fixture.state();
    assert!(state.litellm_params.get("async_call").is_none());
    assert!(state.litellm_params.get("litellm_logging_obj").is_none());
}

#[test]
fn create_extra_query_never_reaches_the_handler_or_the_logging() {
    let fixture = Fixture::new();
    let mut call = create_call();
    call.extra_query = Some(query());

    fixture.create(&call).expect("the handler answers");

    let creates = fixture.handler.creates();
    assert!(creates[0].params.get("api-version").is_none());
    assert!(creates[0].params.get("z").is_none());
    assert!(creates[0].base.litellm_params.get("api-version").is_none());
    assert!(creates[0].base.litellm_params.get("z").is_none());
    let state = fixture.state();
    assert!(state.litellm_params.get("api-version").is_none());
    assert!(state.litellm_params.get("z").is_none());
    assert!(state.optional_params.get("api-version").is_none());
}

#[test]
fn create_a_direct_call_without_routing_hints_keeps_the_handler_id() {
    let fixture = Fixture::new();

    let container = fixture.create(&create_call()).expect("the handler answers");

    assert_eq!(container.id, CONTAINER_ID);
    assert_eq!(container.status, "running");
    assert_eq!(container.name.as_deref(), Some("store"));
    assert_eq!(fixture.handler.creates().len(), 1);
}

#[test]
fn create_encodes_the_id_from_litellm_metadata() {
    let fixture = Fixture::new();
    let mut call = create_call();
    call.litellm_metadata = Some(routing_metadata("gpt-4o"));

    let container = fixture.create(&call).expect("the handler answers");

    assert_eq!(
        container.id,
        build_container_id(Some("openai"), Some("gpt-4o"), CONTAINER_ID)
    );
    assert_eq!(container.status, "running");
    assert_eq!(container.created_at, 3);
    assert_eq!(container.name.as_deref(), Some("store"));
}

#[test]
fn create_extra_body_encodes_the_id_but_never_reaches_the_handler() {
    let fixture = Fixture::new();
    let mut call = create_call();
    call.extra_body = Some(BTreeMap::from([(
        "target_model_names".to_owned(),
        "gpt-4o-mini, gpt-4o".to_owned(),
    )]));

    let container = fixture.create(&call).expect("the handler answers");

    assert_eq!(
        container.id,
        build_container_id(Some("openai"), Some("gpt-4o-mini"), CONTAINER_ID)
    );
    let creates = fixture.handler.creates();
    assert!(creates[0].params.get("extra_body").is_none());
    assert!(creates[0].params.get("target_model_names").is_none());
    assert!(creates[0].base.litellm_params.get("extra_body").is_none());
    let state = fixture.state();
    assert!(state.optional_params.get("extra_body").is_none());
    assert!(state.litellm_params.get("extra_body").is_none());
}

#[test]
fn create_an_unusable_metadata_field_is_a_bad_request() {
    let fixture = Fixture::new();
    let mut call = create_call();
    call.litellm_metadata = Some(IndexMap::from([(
        "model_info".to_owned(),
        JsonValue::Object(IndexMap::from([(
            "id".to_owned(),
            JsonValue::Number(Number::from(5_i64)),
        )])),
    )]));

    let failure = fixture.create(&call).expect_err("the id is not text");

    let ErrorClass::BadRequest(details) = &failure.class else {
        panic!("expected a bad request, got {:?}", failure.class);
    };
    assert_eq!(details.status.0, 400);
    assert_eq!(
        details.body_param.as_deref(),
        Some("litellm_metadata.model_info.id")
    );
    assert_eq!(details.model.as_deref(), Some(""));
    assert_eq!(details.provider.as_deref(), Some("openai"));
    assert!(details.message.contains("litellm_metadata.model_info.id"));
    // the encode step runs after the handler answered
    assert_eq!(fixture.handler.creates().len(), 1);
}

#[test]
fn create_a_provider_error_keeps_its_status() {
    let failure = handler_failure(ContainerCollectionError::Provider(provider_error(
        404,
        "No such container: cntr_1",
    )));

    let ErrorClass::NotFound(details) = &failure.class else {
        panic!("expected a not found, got {:?}", failure.class);
    };
    assert_eq!(details.status.0, 404);
    assert_eq!(details.provider.as_deref(), Some("openai"));
    assert_eq!(details.model.as_deref(), Some(""));
    assert!(details.message.contains("No such container: cntr_1"));
}

#[test]
fn create_a_config_upstream_error_keeps_its_status() {
    let failure = handler_failure(ContainerCollectionError::Config(ContainerError::Upstream(
        provider_error(429, "slow down"),
    )));

    let ErrorClass::RateLimit(details) = &failure.class else {
        panic!("expected a rate limit, got {:?}", failure.class);
    };
    assert_eq!(details.context.provider.as_deref(), Some("openai"));
    assert!(details.context.message.contains("slow down"));
}

#[test]
fn create_a_wrongly_typed_handler_param_names_the_field() {
    let failure = handler_failure(ContainerCollectionError::Params(
        ContainerFileHttpError::ParamType {
            field: "ssl_verify",
            expected: "a boolean or a CA bundle path",
        },
    ));

    let ErrorClass::BadRequest(details) = &failure.class else {
        panic!("expected a bad request, got {:?}", failure.class);
    };
    assert_eq!(details.status.0, 400);
    assert_eq!(details.body_param.as_deref(), Some("ssl_verify"));
    assert!(details.message.contains("ssl_verify"));
}

#[test]
fn create_a_closed_client_pool_is_a_value_error() {
    let failure = handler_failure(ContainerCollectionError::Client(PoolError::Closed));

    let ErrorClass::ApiConnection(context) = &failure.class else {
        panic!("expected an api connection, got {:?}", failure.class);
    };
    assert_eq!(context.provider.as_deref(), Some("openai"));
    assert!(context.message.contains("the pool is closed"));
}

#[test]
fn create_a_logging_failure_is_a_value_error() {
    let failure = handler_failure(ContainerCollectionError::Logging(
        PrePostError::RawRequestCaptured,
    ));

    assert!(matches!(&failure.class, ErrorClass::ApiConnection(_)));
}

#[test]
fn create_an_encoding_failure_is_a_value_error() {
    let source = serde_json::from_str::<Json>("{ ").expect_err("the fixture is not JSON");
    let failure = handler_failure(ContainerCollectionError::Encode(source));

    assert!(matches!(&failure.class, ErrorClass::ApiConnection(_)));
}

#[test]
fn create_a_suspended_handler_is_a_value_error() {
    let failure = handler_failure(ContainerCollectionError::Suspended);

    assert!(matches!(&failure.class, ErrorClass::ApiConnection(_)));
}

#[test]
fn create_a_config_failure_without_an_upstream_answer_is_a_value_error() {
    let failure = handler_failure(ContainerCollectionError::Config(
        ContainerError::ApiBaseRequired,
    ));

    assert!(matches!(&failure.class, ErrorClass::ApiConnection(_)));
}

#[test]
fn create_a_handler_params_failure_that_is_not_a_param_type_is_a_value_error() {
    let failure = handler_failure(ContainerCollectionError::Params(
        ContainerFileHttpError::Suspended,
    ));

    assert!(matches!(&failure.class, ErrorClass::ApiConnection(_)));
}
