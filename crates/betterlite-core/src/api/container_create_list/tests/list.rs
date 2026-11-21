#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_error::error::ErrorClass;
use betterlite_http::proxy::pool::PoolError;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::api::container_contracts::error::BaseLlmException;
use betterlite_openai::containers::container_file_http::ContainerFileHttpError;
use indexmap::IndexMap;
use serde_json::Number;

use super::super::ContainerListCall;
use super::super::error::ContainerOpFailure;
use super::support::{
    API_BASE, API_KEY, API_VERSION, CALL_ID, CONTAINER_ID, Fixture, HandlerForm, INITIAL_CALL_TYPE,
    StubLookup, declared_timeout, headers, list_call, object, query, request_timeout,
    routing_metadata, secret,
};
use crate::api::container_collection_http::ContainerCollectionError;

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
        .list(&list_call())
        .expect_err("the handler failed the call")
}

fn paginated_call() -> ContainerListCall {
    let mut call = list_call();
    call.after = Some("cntr_0".to_owned());
    call.limit = Some(5);
    call.order = Some("asc".to_owned());
    call
}

#[test]
fn list_hands_the_pagination_headers_and_extra_query_to_the_handler() {
    let fixture = Fixture::new();
    let mut call = paginated_call();
    call.extra_headers = Some(headers());
    call.extra_query = Some(query());

    fixture.list(&call).expect("the handler answers");

    let lists = fixture.handler.lists();
    assert_eq!(lists.len(), 1);
    assert_eq!(lists[0].base.form, HandlerForm::Blocking);
    assert_eq!(lists[0].after.as_deref(), Some("cntr_0"));
    assert_eq!(lists[0].limit, Some(5));
    assert_eq!(lists[0].order.as_deref(), Some("asc"));
    assert_eq!(lists[0].base.headers, Some(headers()));
    // the handler receives the caller's values, text and integers as given
    assert_eq!(
        lists[0].extra_query,
        Some(object(r#"{"api-version":"v1","z":2}"#))
    );
}

#[test]
fn list_logs_the_requested_params_with_the_list_call_type() {
    let fixture = Fixture::new();
    let mut call = paginated_call();
    call.litellm_call_id = Some(CALL_ID.to_owned());

    fixture.list(&call).expect("the handler answers");

    let state = fixture.state();
    assert_eq!(state.call_type, "list_containers");
    assert_eq!(state.model, "");
    assert_eq!(state.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(
        state.litellm_params.get("litellm_call_id"),
        Some(&Json::Text(CALL_ID.to_owned()))
    );
    assert_eq!(
        state.litellm_params.get("after"),
        Some(&Json::Text("cntr_0".to_owned()))
    );
}

#[test]
fn list_logs_the_pagination_params_it_was_given() {
    let fixture = Fixture::new();
    let call = paginated_call();

    fixture.list(&call).expect("the handler answers");

    let state = fixture.state();
    assert_eq!(
        state.optional_params.get("after"),
        Some(&Json::Text("cntr_0".to_owned()))
    );
    assert_eq!(
        state.optional_params.get("limit"),
        Some(&Json::Number(Number::from(5_i64)))
    );
    assert_eq!(
        state.optional_params.get("order"),
        Some(&Json::Text("asc".to_owned()))
    );
}

#[test]
fn list_logs_the_extra_headers_and_extra_query_but_never_extra_body() {
    let fixture = Fixture::new();
    let mut call = paginated_call();
    call.extra_headers = Some(headers());
    call.extra_query = Some(query());
    call.extra_body = Some(BTreeMap::from([(
        "target_model_names".to_owned(),
        "gpt-4o-mini".to_owned(),
    )]));

    fixture.list(&call).expect("the handler answers");

    let state = fixture.state();
    assert_eq!(
        state.optional_params.get("extra_headers"),
        Some(&Json::Object(IndexMap::from([(
            "X-Test".to_owned(),
            Json::Text("1".to_owned()),
        )])))
    );
    // the record's extra_query carries text, so the logged query is text too
    assert_eq!(
        state.optional_params.get("extra_query"),
        Some(&Json::Object(IndexMap::from([
            ("api-version".to_owned(), Json::Text("v1".to_owned())),
            ("z".to_owned(), Json::Text("2".to_owned())),
        ])))
    );
    assert!(state.optional_params.get("extra_body").is_none());
}

#[test]
fn list_the_handler_sees_the_logging_state_with_the_call_type_set() {
    let fixture = Fixture::new();

    fixture.list(&list_call()).expect("the handler answers");

    let lists = fixture.handler.lists();
    assert_eq!(lists[0].base.call_type, "list_containers");
    assert_eq!(lists[0].base.provider.as_deref(), Some("openai"));
    assert_eq!(lists[0].base.model, "");
    assert!(lists[0].base.optional_params.is_empty());
}

#[test]
fn list_returns_the_handler_response_with_its_ids_untouched() {
    let fixture = Fixture::new();
    let mut call = list_call();
    call.litellm_metadata = Some(routing_metadata("gpt-4o"));
    call.extra_body = Some(BTreeMap::from([(
        "target_model_names".to_owned(),
        "gpt-4o-mini".to_owned(),
    )]));

    let response = fixture.list(&call).expect("the handler answers");

    assert_eq!(response.data.len(), 1);
    assert_eq!(response.data[0].id, CONTAINER_ID);
    assert_eq!(response.first_id.as_deref(), Some(CONTAINER_ID));
    assert!(!response.has_more);
    assert_eq!(fixture.handler.lists().len(), 1);
}

#[test]
fn list_extra_body_never_reaches_the_handler_or_the_logging() {
    let fixture = Fixture::new();
    let mut call = list_call();
    call.extra_body = Some(BTreeMap::from([(
        "target_model_names".to_owned(),
        "gpt-4o-mini".to_owned(),
    )]));

    fixture.list(&call).expect("the handler answers");

    let lists = fixture.handler.lists();
    assert!(lists[0].base.litellm_params.get("extra_body").is_none());
    assert!(
        lists[0]
            .base
            .litellm_params
            .get("target_model_names")
            .is_none()
    );
    let state = fixture.state();
    assert!(state.optional_params.get("extra_body").is_none());
    assert!(state.litellm_params.get("extra_body").is_none());
}

#[test]
fn list_defaults_to_the_openai_provider() {
    let fixture = Fixture::new();
    let call = list_call();

    fixture.list(&call).expect("the handler answers");

    assert_eq!(call.custom_llm_provider, "openai");
    assert_eq!(fixture.lookup.asked(), vec![ProviderId::Openai]);
}

#[test]
fn list_azure_resolves_to_the_azure_config() {
    let fixture = Fixture::new();
    let mut call = list_call();
    call.custom_llm_provider = "azure".to_owned();

    fixture.list(&call).expect("the handler answers");

    assert_eq!(fixture.lookup.asked(), vec![ProviderId::Azure]);
    assert_eq!(
        fixture.state().custom_llm_provider.as_deref(),
        Some("azure")
    );
}

#[test]
fn list_a_known_provider_without_a_container_config_is_missing() {
    let fixture = Fixture::new();
    let mut call = list_call();
    call.custom_llm_provider = "anthropic".to_owned();

    let failure = fixture
        .list(&call)
        .expect_err("no container config answers anthropic");

    assert_eq!(
        failure.error.to_string(),
        "Container provider config not found for provider: anthropic"
    );
    let ErrorClass::ApiConnection(context) = &failure.class else {
        panic!("expected an api connection, got {:?}", failure.class);
    };
    assert_eq!(context.provider.as_deref(), Some("anthropic"));
    assert_eq!(context.model.as_deref(), Some(""));
    assert!(fixture.lookup.asked().is_empty());
    assert!(fixture.handler.lists().is_empty());
    assert_eq!(fixture.state().call_type, INITIAL_CALL_TYPE);
}

#[test]
fn list_a_provider_betterlite_does_not_know_is_a_value_error() {
    let fixture = Fixture::new();
    let mut call = list_call();
    call.custom_llm_provider = "acme".to_owned();

    let failure = fixture.list(&call).expect_err("acme is no provider");

    assert!(failure.error.to_string().contains("acme"));
    assert!(matches!(&failure.class, ErrorClass::ApiConnection(_)));
    assert!(fixture.lookup.asked().is_empty());
    assert!(fixture.handler.lists().is_empty());
}

#[test]
fn list_a_provider_config_the_registry_lacks_is_missing() {
    let mut fixture = Fixture::new();
    fixture.lookup = StubLookup::empty();

    let failure = fixture
        .list(&list_call())
        .expect_err("the registry answers no config");

    assert_eq!(
        failure.error.to_string(),
        "Container provider config not found for provider: openai"
    );
    assert_eq!(fixture.lookup.asked(), vec![ProviderId::Openai]);
    assert!(fixture.handler.lists().is_empty());
}

#[test]
fn list_uses_the_default_call_timeout_of_six_hundred_seconds() {
    let fixture = Fixture::new();

    fixture.list(&list_call()).expect("the handler answers");

    let lists = fixture.handler.lists();
    assert_eq!(lists[0].base.timeout, Some(declared_timeout()));
}

#[test]
fn list_an_absent_timeout_falls_back_to_the_request_timeout() {
    let fixture = Fixture::new();
    let mut call = list_call();
    call.timeout = None;

    fixture.list(&call).expect("the handler answers");

    let lists = fixture.handler.lists();
    assert_eq!(lists[0].base.timeout, Some(request_timeout()));
}

#[test]
fn list_a_declared_timeout_reaches_the_handler() {
    let fixture = Fixture::new();
    let mut call = list_call();
    call.timeout = Some(5.0_f64);

    fixture.list(&call).expect("the handler answers");

    let lists = fixture.handler.lists();
    assert_eq!(lists[0].base.timeout, Some(Duration::from_secs(5)));
}

#[test]
fn list_a_zero_timeout_falls_back_to_the_request_timeout() {
    let fixture = Fixture::new();
    let mut call = list_call();
    call.timeout = Some(0.0_f64);

    fixture.list(&call).expect("the handler answers");

    let lists = fixture.handler.lists();
    assert_eq!(lists[0].base.timeout, Some(request_timeout()));
}

#[test]
fn list_a_negative_timeout_is_a_value_error() {
    let fixture = Fixture::new();
    let mut call = list_call();
    call.timeout = Some(-1.0_f64);

    let failure = fixture.list(&call).expect_err("the timeout is unusable");

    let ErrorClass::ApiConnection(context) = &failure.class else {
        panic!("expected an api connection, got {:?}", failure.class);
    };
    assert_eq!(context.provider.as_deref(), Some("openai"));
    assert!(context.message.contains("is not a usable duration"));
    assert!(fixture.handler.lists().is_empty());
}

#[test]
fn list_passes_credentials_and_extra_kwargs_to_the_handler() {
    let fixture = Fixture::new();
    let mut call = list_call();
    call.api_key = Some(secret(API_KEY));
    call.api_base = Some(API_BASE.to_owned());
    call.api_version = Some(API_VERSION.to_owned());
    call.litellm_call_id = Some(CALL_ID.to_owned());
    call.litellm_metadata = Some(routing_metadata("gpt-4o"));
    call.extra.insert("ssl_verify".to_owned(), Json::Bool(true));

    fixture.list(&call).expect("the handler answers");

    let lists = fixture.handler.lists();
    assert_eq!(
        lists[0].base.litellm_params,
        object(concat!(
            r#"{"api_base":"https://api.openai.com/v1","api_key":"sk-test","#,
            r#""api_version":"2024-06-01","litellm_call_id":"call-1","#,
            r#""litellm_metadata":{"model_info":{"id":"gpt-4o"}},"ssl_verify":true}"#,
        ))
    );
}

#[test]
fn list_a_provider_error_keeps_its_status() {
    let failure = handler_failure(ContainerCollectionError::Provider(provider_error(
        401,
        "invalid api key",
    )));

    let ErrorClass::Authentication(details) = &failure.class else {
        panic!("expected an authentication error, got {:?}", failure.class);
    };
    assert_eq!(details.status.0, 401);
    assert_eq!(details.provider.as_deref(), Some("openai"));
    assert_eq!(details.model.as_deref(), Some(""));
}

#[test]
fn list_a_wrongly_typed_handler_param_names_the_field() {
    let failure = handler_failure(ContainerCollectionError::Params(
        ContainerFileHttpError::ParamType {
            field: "limit",
            expected: "an integer",
        },
    ));

    let ErrorClass::BadRequest(details) = &failure.class else {
        panic!("expected a bad request, got {:?}", failure.class);
    };
    assert_eq!(details.body_param.as_deref(), Some("limit"));
    assert!(details.message.contains("limit"));
}

#[test]
fn list_a_closed_client_pool_is_a_value_error() {
    let failure = handler_failure(ContainerCollectionError::Client(PoolError::Closed));

    assert!(matches!(&failure.class, ErrorClass::ApiConnection(_)));
}
