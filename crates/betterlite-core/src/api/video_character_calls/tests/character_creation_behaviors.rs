#![cfg(test)]

use std::time::Duration;

use betterlite_error::error::ErrorClass;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_openai::video_edits::response::CharacterKind;

use super::support::{
    CALL_ID, Fixture, INITIAL_CALL_TYPE, MOCK_JSON, REQUEST_TIMEOUT_SECONDS, Reply, create_call,
    headers, json, video,
};
use crate::api::bfl::edit::media::SuppliedMedia;
use crate::api::video_character_calls::{CharacterOutcome, DEFAULT_TIMEOUT_SECONDS};

fn name_params(name: &str) -> indexmap::IndexMap<String, Json> {
    indexmap::IndexMap::from([("name".to_owned(), Json::Text(name.to_owned()))])
}

#[test]
fn video_create_character_mock_string_and_object() {
    let mut from_string = create_call();
    from_string.name = "Mocked".to_owned();
    from_string.mock_response = Some(Json::Text(MOCK_JSON.to_owned()));
    let mut fixture = Fixture::new();
    let character = match fixture.create(&from_string).expect("the mock answers") {
        CharacterOutcome::Done(character) => character,
        CharacterOutcome::Pending(_) => panic!("a mock answers directly"),
    };
    assert_eq!(character.id, "char-9");
    assert_eq!(character.name, "Mocked");
    assert_eq!(character.created_at, 42);
    assert_eq!(character.object, CharacterKind::Character);
    assert!(fixture.handler.creates.is_empty());
    assert_eq!(fixture.state().call_type, INITIAL_CALL_TYPE);
    assert!(fixture.state().optional_params.is_empty());

    let mut from_object = create_call();
    from_object.name = "Mocked".to_owned();
    from_object.mock_response = Some(json(MOCK_JSON));
    let mut object_fixture = Fixture::answering(Reply::Failed {
        status_code: Some(500),
        message: "the handler must not run",
    });
    let object_character = match object_fixture
        .create(&from_object)
        .expect("the mock answers")
    {
        CharacterOutcome::Done(answered) => answered,
        CharacterOutcome::Pending(_) => panic!("a mock answers directly"),
    };
    assert_eq!(object_character.name, "Mocked");
    assert!(object_fixture.handler.creates.is_empty());
}

#[test]
fn video_create_character_openai_fallback() {
    let mut fixture = Fixture::new();
    let call = create_call();

    let outcome = fixture.create(&call).expect("the handler answers");

    let CharacterOutcome::Done(character) = outcome else {
        panic!("a direct answer is done");
    };
    assert_eq!(character, super::support::character());
    let seen = &fixture.handler.creates[0];
    assert_eq!(seen.provider, "openai");
    assert_eq!(seen.name, "Wren");
    assert_eq!(seen.video, video());
    assert!(!seen.is_async);
    assert_eq!(seen.timeout, Duration::from_secs(600));
    assert_eq!(DEFAULT_TIMEOUT_SECONDS, 600.0_f64);
}

#[tokio::test]
async fn avideo_create_character_defaults_and_openai() {
    let mut fixture = Fixture::new();
    let call = create_call();

    let character = fixture.acreate(&call).await.expect("the handler answers");

    assert_eq!(character, super::support::character());
    let seen = &fixture.handler.creates[0];
    assert_eq!(seen.provider, "openai");
    assert!(seen.is_async);
    assert_eq!(seen.timeout, Duration::from_secs(600));
}

#[tokio::test]
async fn avideo_create_character_context_propagation() {
    let mut fixture = Fixture::answering(Reply::Deferred);
    let call = create_call();

    let character = fixture.acreate(&call).await.expect("the future answers");

    assert_eq!(character, super::support::character());
    let seen = &fixture.handler.creates[0];
    assert_eq!(seen.logged.model, "");
    assert_eq!(seen.logged.call_type, "video_create_character");
    assert_eq!(seen.logged.user.as_deref(), Some("max"));
    assert_eq!(seen.logged.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(seen.logged.optional_params, name_params("Wren"));
    assert_eq!(
        seen.logged.litellm_params,
        indexmap::IndexMap::from([
            ("litellm_call_id".to_owned(), Json::Text(CALL_ID.to_owned())),
            ("name".to_owned(), Json::Text("Wren".to_owned())),
        ])
    );
}

#[test]
fn create_character_request_params() {
    let mut fixture = Fixture::new();
    let mut call = create_call();
    call.extra_headers = Some(headers());
    call.extra.insert("mockless".to_owned(), Json::Bool(true));

    fixture.create(&call).expect("the handler answers");

    let params = &fixture.handler.creates[0].litellm_params;
    let names: Vec<&str> = params.keys().map(String::as_str).collect();
    assert_eq!(
        names,
        vec!["api_base", "api_key", "litellm_call_id", "mockless", "user"]
    );
    assert_eq!(
        params.get("api_key"),
        Some(&JsonValue::Text("sk-test".to_owned()))
    );
    assert_eq!(
        params.get("litellm_call_id"),
        Some(&JsonValue::Text(CALL_ID.to_owned()))
    );
    assert!(!params.contains_key("name"));
    assert!(!params.contains_key("video"));
    assert!(!params.contains_key("timeout"));
    assert!(!params.contains_key("custom_llm_provider"));
    assert_eq!(fixture.handler.creates[0].extra_headers, Some(headers()));
    assert_eq!(
        fixture.handler.creates[0].logged.optional_params,
        name_params("Wren")
    );
}

#[test]
fn create_character_logging_updates() {
    let mut fixture = Fixture::new();
    let call = create_call();

    fixture.create(&call).expect("the handler answers");

    let state = fixture.state();
    assert_eq!(state.model, "");
    assert_eq!(state.call_type, "video_create_character");
    assert_eq!(state.user.as_deref(), Some("max"));
    assert_eq!(state.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(state.optional_params, name_params("Wren"));
    assert_eq!(
        state.litellm_params,
        indexmap::IndexMap::from([
            ("litellm_call_id".to_owned(), Json::Text(CALL_ID.to_owned())),
            ("name".to_owned(), Json::Text("Wren".to_owned())),
        ])
    );
}

#[test]
fn create_a_provider_without_a_video_config_is_unsupported() {
    let mut fixture = Fixture::new();
    let mut call = create_call();
    call.custom_llm_provider = "anthropic".to_owned();

    let failure = fixture.create(&call).expect_err("no video config answers");

    assert_eq!(
        failure.error.to_string(),
        "video create character is not supported for anthropic"
    );
    let ErrorClass::ApiConnection(context) = &failure.class else {
        panic!("expected an api connection, got {:?}", failure.class);
    };
    assert_eq!(context.provider.as_deref(), Some("anthropic"));
    assert_eq!(context.model.as_deref(), Some(""));
    assert!(fixture.handler.creates.is_empty());
    assert_eq!(fixture.state().call_type, INITIAL_CALL_TYPE);
}

#[test]
fn create_a_provider_betterlite_does_not_know_is_a_value_error() {
    let mut fixture = Fixture::new();
    let mut call = create_call();
    call.custom_llm_provider = "acme".to_owned();

    let failure = fixture.create(&call).expect_err("acme is no provider");

    assert!(failure.error.to_string().contains("acme"));
    let ErrorClass::ApiConnection(context) = &failure.class else {
        panic!("expected an api connection, got {:?}", failure.class);
    };
    assert_eq!(context.provider.as_deref(), Some("acme"));
    assert!(fixture.handler.creates.is_empty());
}

#[test]
fn create_edenai_is_unavailable_until_its_config_is_installed() {
    let mut fixture = Fixture::new();
    let mut call = create_call();
    call.custom_llm_provider = "edenai".to_owned();

    let failure = fixture
        .create(&call)
        .expect_err("B5838 installs the config");

    assert_eq!(
        failure.error.to_string(),
        "no video config is installed for 'edenai'"
    );
    let ErrorClass::ApiConnection(context) = &failure.class else {
        panic!("expected an api connection, got {:?}", failure.class);
    };
    assert_eq!(context.provider.as_deref(), Some("edenai"));
    assert!(fixture.handler.creates.is_empty());
}

#[test]
fn create_falsey_timeout_falls_back_to_the_request_timeout() {
    for timeout in [Some(0.0_f64), None] {
        let mut fixture = Fixture::new();
        let mut call = create_call();
        call.timeout = timeout;

        fixture.create(&call).expect("the handler answers");

        assert_eq!(
            fixture.handler.creates[0].timeout,
            Duration::from_millis(1_234_000)
        );
        assert_eq!(REQUEST_TIMEOUT_SECONDS, 1234.0_f64);
    }
}

#[test]
fn create_truthy_timeout_passes_through() {
    let mut fixture = Fixture::new();
    let mut call = create_call();
    call.timeout = Some(2.5_f64);

    fixture.create(&call).expect("the handler answers");

    assert_eq!(
        fixture.handler.creates[0].timeout,
        Duration::from_millis(2500)
    );
}

#[test]
fn create_handler_failure_with_status_keeps_its_class() {
    let mut fixture = Fixture::answering(Reply::Failed {
        status_code: Some(429),
        message: "slow down",
    });

    let failure = fixture
        .create(&create_call())
        .expect_err("the handler fails");

    let ErrorClass::RateLimit(details) = &failure.class else {
        panic!("expected a rate limit, got {:?}", failure.class);
    };
    assert_eq!(details.context.provider.as_deref(), Some("openai"));
    assert_eq!(details.context.model.as_deref(), Some(""));
    assert!(details.context.message.contains("slow down"));
    assert_eq!(failure.error.to_string(), "slow down");
}

#[test]
fn create_handler_failure_without_status_is_an_api_connection() {
    let mut fixture = Fixture::answering(Reply::Failed {
        status_code: None,
        message: "the client broke",
    });

    let failure = fixture
        .create(&create_call())
        .expect_err("the handler fails");

    let ErrorClass::ApiConnection(context) = &failure.class else {
        panic!("expected an api connection, got {:?}", failure.class);
    };
    assert_eq!(context.provider.as_deref(), Some("openai"));
    assert!(context.message.contains("the client broke"));
}

#[test]
fn create_a_wrongly_typed_user_is_a_bad_request() {
    let mut fixture = Fixture::new();
    let mut call = create_call();
    call.extra.insert(
        "user".to_owned(),
        Json::Number(serde_json::Number::from(5_i64)),
    );

    let failure = fixture.create(&call).expect_err("the user is not text");

    let ErrorClass::BadRequest(details) = &failure.class else {
        panic!("expected a bad request, got {:?}", failure.class);
    };
    assert_eq!(details.status.0, 400);
    assert_eq!(details.body_param.as_deref(), Some("user"));
    assert_eq!(details.model.as_deref(), Some(""));
    assert_eq!(details.provider.as_deref(), Some("openai"));
    assert!(details.message.contains("user"));
    assert!(fixture.handler.creates.is_empty());
}

#[test]
fn create_mock_invalid_json_is_a_value_error() {
    let mut fixture = Fixture::new();
    let mut call = create_call();
    call.mock_response = Some(Json::Text("not json".to_owned()));

    let failure = fixture.create(&call).expect_err("the mock is no JSON");

    assert_eq!(
        failure.error.to_string(),
        "the mock response is not valid JSON"
    );
    let ErrorClass::ApiConnection(context) = &failure.class else {
        panic!("expected an api connection, got {:?}", failure.class);
    };
    assert_eq!(context.provider.as_deref(), Some("openai"));
    assert!(fixture.handler.creates.is_empty());
}

#[test]
fn create_mock_with_the_wrong_shape_is_a_value_error() {
    let mut fixture = Fixture::new();
    let mut call = create_call();
    call.mock_response = Some(json(r#"{"id":1}"#));

    let failure = fixture.create(&call).expect_err("the id is not text");

    assert!(
        failure
            .error
            .to_string()
            .contains("the mock response does not match a character object")
    );
    assert!(matches!(failure.class, ErrorClass::ApiConnection(_)));
    assert!(fixture.handler.creates.is_empty());
}

#[test]
fn create_mock_answers_before_the_provider_is_read() {
    let mut fixture = Fixture::new();
    let mut call = create_call();
    call.custom_llm_provider = "acme".to_owned();
    call.mock_response = Some(json(MOCK_JSON));

    let outcome = fixture.create(&call).expect("the mock answers first");

    let CharacterOutcome::Done(character) = outcome else {
        panic!("a mock answers directly");
    };
    assert_eq!(character.name, "Mocked");
    assert!(fixture.handler.creates.is_empty());
}

#[tokio::test]
async fn create_with_the_async_flag_returns_the_pending_future() {
    let mut fixture = Fixture::answering(Reply::Deferred);
    let mut call = create_call();
    call.is_async = true;

    let outcome = fixture.create(&call).expect("the handler raises a future");
    let CharacterOutcome::Pending(pending) = outcome else {
        panic!("the async flag asks for the future");
    };
    let character = pending.await.expect("the future answers");
    assert_eq!(character, super::support::character());
    assert!(fixture.handler.creates[0].is_async);
}

#[tokio::test]
async fn create_a_pending_failure_is_mapped() {
    let mut fixture = Fixture::answering(Reply::DeferredFailure {
        status_code: Some(400),
        message: "the future failed",
    });
    let mut call = create_call();
    call.is_async = true;

    let outcome = fixture.create(&call).expect("the handler raises a future");
    let CharacterOutcome::Pending(pending) = outcome else {
        panic!("the async flag asks for the future");
    };
    let failure = pending.await.expect_err("the future fails");

    let ErrorClass::BadRequest(details) = &failure.class else {
        panic!("expected a bad request, got {:?}", failure.class);
    };
    assert_eq!(details.status.0, 400);
    assert!(details.message.contains("the future failed"));
}

#[test]
fn create_extra_headers_and_the_video_reach_the_handler() {
    let mut fixture = Fixture::new();
    let mut call = create_call();
    call.extra_headers = Some(headers());

    fixture.create(&call).expect("the handler answers");

    let seen = &fixture.handler.creates[0];
    assert_eq!(seen.extra_headers, Some(headers()));
    assert_eq!(seen.video, SuppliedMedia::Bytes(b"video-bytes".to_vec()));
}

// the outcome's Debug names its form, since a pending answer's future cannot be printed
#[test]
fn create_outcome_debug_names_the_form() {
    let mut deferred = Fixture::answering(Reply::Deferred);
    let mut call = create_call();
    call.is_async = true;
    let pending = deferred.create(&call).expect("the handler raises a future");
    assert_eq!(format!("{pending:?}"), "Pending(<future>)");

    let mut direct = Fixture::new();
    let done = direct.create(&create_call()).expect("the handler answers");
    assert!(format!("{done:?}").starts_with("Done("));
}

// kwargs.get("mock_response", None) is None for an explicit null, so the provider runs
#[test]
fn create_a_null_mock_response_is_absent() {
    let mut fixture = Fixture::new();
    let mut call = create_call();
    call.mock_response = Some(Json::Null);

    let outcome = fixture.create(&call).expect("the handler answers");

    let CharacterOutcome::Done(character) = outcome else {
        panic!("the handler answers directly");
    };
    assert_eq!(character, super::support::character());
    assert_eq!(fixture.handler.creates.len(), 1);
}

// a timeout that is no usable duration is refused after the logging update and before the handler
#[test]
fn create_an_unusable_timeout_is_refused_after_logging() {
    let mut fixture = Fixture::new();
    let mut call = create_call();
    call.timeout = Some(-1.0_f64);

    let failure = fixture
        .create(&call)
        .expect_err("a negative deadline is unusable");

    assert_eq!(
        failure.error.to_string(),
        "the request timeout of -1 seconds is not a usable duration"
    );
    assert!(matches!(failure.class, ErrorClass::ApiConnection(_)));
    assert!(fixture.handler.creates.is_empty());
    assert_eq!(fixture.state().call_type, "video_create_character");
}

// extra_query and extra_body are accepted and never forwarded on a character call
#[test]
fn create_extra_query_and_body_are_not_forwarded() {
    let mut fixture = Fixture::new();
    let mut call = create_call();
    call.extra_query = Some(indexmap::IndexMap::from([(
        "api-version".to_owned(),
        Json::Text("2024-01-01".to_owned()),
    )]));
    call.extra_body = Some(indexmap::IndexMap::from([(
        "metadata".to_owned(),
        Json::Bool(true),
    )]));

    fixture.create(&call).expect("the handler answers");

    let params = &fixture.handler.creates[0].litellm_params;
    assert!(!params.contains_key("api-version"));
    assert!(!params.contains_key("metadata"));
}
