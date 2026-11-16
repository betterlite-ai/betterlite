use std::time::Duration;

use betterlite_config::tls::Verification;
use betterlite_interactions::types::interaction::InteractionInput;
use betterlite_models::controls::json_value::JsonValue;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::support::{
    Call, Fixture, FixtureError, api_key_header, clients, dead_address, serve_with_body,
};
use crate::interactions::base::clients::HttpClients;
use crate::interactions::base::tests::support as base;
use crate::interactions::base::{ClientError, Logging, Params, RequestFailure};
use crate::interactions::create::asynchronous::async_create_interaction;
use crate::interactions::create::{CreateInteractionError, CreateOutcome};
use crate::proxy::base_builder;

fn posted(body: &str) -> JsonValue {
    serde_json::from_str(body).unwrap()
}

#[tokio::test]
async fn async_create_interaction_posts_the_transformed_request_and_returns_the_response() {
    let serving = serve_with_body(base::reply("200 OK", &[], "{\"id\":\"int_1\"}"), None);
    let fixture = Fixture {
        headers: api_key_header(),
        data: base::params(r#"{"model": "gemini-2.5-flash", "input": "hello"}"#),
        ..Fixture::new()
    };
    let mut call = Call::new(serving.address);
    call.params.insert(
        "custom_llm_provider".to_owned(),
        JsonValue::Text("gemini".to_owned()),
    );
    let mut clients = clients();
    let mut logging = Logging::default();

    let outcome =
        async_create_interaction(&fixture, &mut clients, &mut logging, None, &call.request())
            .await
            .unwrap();

    let CreateOutcome::Response(body) = outcome else {
        panic!("a non-stream request returns the response");
    };
    assert_eq!(body, "{\"id\":\"int_1\"}");
    let received = serving.request.join().unwrap();
    assert!(
        received
            .head
            .starts_with("POST /v1beta/interactions HTTP/1.1"),
        "{}",
        received.head
    );
    assert!(received.head.contains("x-goog-api-key: key-1\r\n"));
    assert_eq!(
        posted(&received.body),
        posted(r#"{"model": "gemini-2.5-flash", "input": "hello"}"#)
    );
}

#[tokio::test]
async fn async_create_interaction_returns_the_open_response_as_a_stream_iterator() {
    let streaming = "data: {\"event_type\": \"step.start\"}\n\ndata: [DONE]\n\n";
    let serving = serve_with_body(base::reply("200 OK", &[], streaming), None);
    let fixture = Fixture {
        headers: api_key_header(),
        ..Fixture::new()
    };
    let mut call = Call::new(serving.address);
    call.stream = true;
    let mut clients = clients();
    let mut logging = Logging::default();

    let outcome =
        async_create_interaction(&fixture, &mut clients, &mut logging, None, &call.request())
            .await
            .unwrap();

    let CreateOutcome::Stream(source) = outcome else {
        panic!("a stream request returns the iterator");
    };
    // the iterator receives the body unread, so this reads what its first step would have read
    let body = source.response.text().await.unwrap();
    assert_eq!(body, streaming);
    let calls = fixture.calls();
    assert!(
        calls.contains(&"async_stream|200|gemini-2.5-flash".to_owned()),
        "{calls:?}"
    );
    assert!(calls.iter().all(|call| !call.starts_with("response|")));
    assert_eq!(
        logging.model_call_details.get("streamed"),
        Some(&JsonValue::Bool(true))
    );
    serving.request.join().unwrap();
}

#[tokio::test]
async fn async_create_interaction_converts_a_failing_status_through_the_provider_error_class() {
    let serving = serve_with_body(base::reply("404 Not Found", &[], "missing"), None);
    let fixture = Fixture::new();
    let mut clients = clients();
    let mut logging = Logging::default();

    let error = async_create_interaction(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        &Call::new(serving.address).request(),
    )
    .await
    .err()
    .unwrap();

    assert!(matches!(
        error,
        CreateInteractionError::Provider(FixtureError::Refused(404))
    ));
    let calls = fixture.calls();
    assert!(
        calls.contains(&"error|404|missing|-".to_owned()),
        "{calls:?}"
    );
    assert!(calls.iter().all(|call| !call.starts_with("response|")));
    serving.request.join().unwrap();
}

#[tokio::test]
async fn async_create_interaction_converts_a_failing_status_for_a_stream_request() {
    let serving = serve_with_body(base::reply("500 Internal Server Error", &[], "boom"), None);
    let fixture = Fixture::new();
    let mut call = Call::new(serving.address);
    call.stream = true;
    let mut clients = clients();
    let mut logging = Logging::default();

    let error =
        async_create_interaction(&fixture, &mut clients, &mut logging, None, &call.request())
            .await
            .err()
            .unwrap();

    assert!(matches!(
        error,
        CreateInteractionError::Provider(FixtureError::Refused(500))
    ));
    let calls = fixture.calls();
    assert!(calls.contains(&"error|500|boom|-".to_owned()), "{calls:?}");
    assert!(calls.iter().all(|call| !call.starts_with("async_stream|")));
    serving.request.join().unwrap();
}

#[rstest]
#[case::overlaid(
    Some(Params::from([
        ("input".to_owned(), JsonValue::Text("from-extra".to_owned())),
        ("store".to_owned(), JsonValue::Bool(true)),
    ])),
    r#"{"input": "from-extra", "model": "m", "store": true}"#
)]
#[case::absent(None, r#"{"input": "hello", "model": "m"}"#)]
#[tokio::test]
async fn async_create_interaction_overlays_extra_body_onto_the_transformed_request(
    #[case] extra_body: Option<Params>,
    #[case] expected: &str,
) {
    let serving = serve_with_body(base::reply("200 OK", &[], "{}"), None);
    let fixture = Fixture {
        data: base::params(r#"{"model": "m", "input": "hello"}"#),
        ..Fixture::new()
    };
    let mut call = Call::new(serving.address);
    call.extra_body = extra_body;
    let mut clients = clients();
    let mut logging = Logging::default();

    async_create_interaction(&fixture, &mut clients, &mut logging, None, &call.request())
        .await
        .unwrap();

    let received = serving.request.join().unwrap();
    assert_eq!(posted(&received.body), posted(expected));
    assert_eq!(
        logging.model_call_details.get("complete_input_dict"),
        Some(&posted(expected)),
        "the logged body carries the overlay"
    );
}

#[tokio::test]
async fn async_create_interaction_logs_the_pre_call_with_the_input_and_the_complete_input_dict() {
    let serving = serve_with_body(base::reply("200 OK", &[], "{}"), None);
    let fixture = Fixture {
        headers: api_key_header(),
        data: base::params(r#"{"model": "m"}"#),
        ..Fixture::new()
    };
    let mut call = Call::new(serving.address);
    call.input = Some(InteractionInput::Text("hello".to_owned()));
    let mut clients = clients();
    let mut logging = Logging::default();

    async_create_interaction(&fixture, &mut clients, &mut logging, None, &call.request())
        .await
        .unwrap();

    let recorded = logging.pre_call.as_ref().unwrap();
    assert_eq!(recorded.input, "hello");
    assert_eq!(recorded.api_key.expose_secret(), "");
    assert_eq!(
        recorded.api_base,
        format!("http://{}/v1beta/interactions", serving.address)
    );
    assert_eq!(recorded.headers["x-goog-api-key"].expose_secret(), "key-1");
    assert_eq!(
        logging.model_call_details.get("complete_input_dict"),
        Some(&posted(r#"{"model": "m"}"#))
    );
    serving.request.join().unwrap();
}

#[tokio::test]
async fn async_create_interaction_builds_the_client_before_validating_headers() {
    let fixture = Fixture::new();
    let mut call = Call::new(dead_address());
    call.params
        .insert("ssl_verify".to_owned(), JsonValue::Text("yes".to_owned()));
    let mut clients = clients();
    let mut logging = Logging::default();

    let error =
        async_create_interaction(&fixture, &mut clients, &mut logging, None, &call.request())
            .await
            .err()
            .unwrap();

    assert!(matches!(
        error,
        CreateInteractionError::Client(ClientError::FieldType {
            field: "ssl_verify",
            ..
        })
    ));
    assert!(fixture.calls().is_empty());
}

#[tokio::test]
async fn async_create_interaction_times_out_on_the_request_timeout_when_the_caller_names_none() {
    let serving = serve_with_body(
        base::reply("200 OK", &[], "{}"),
        Some(Duration::from_millis(500)),
    );
    let fixture = Fixture::new();
    let mut call = Call::new(serving.address);
    call.timeout = None;
    call.request_timeout = Duration::from_millis(50);
    let mut clients = clients();
    let mut logging = Logging::default();

    let error =
        async_create_interaction(&fixture, &mut clients, &mut logging, None, &call.request())
            .await
            .err()
            .unwrap();

    assert!(matches!(
        error,
        CreateInteractionError::Transport(RequestFailure::Transport(source)) if source.is_timeout()
    ));
    serving.request.join().unwrap();
}

#[tokio::test]
async fn async_create_interaction_prefers_the_callers_timeout() {
    let serving = serve_with_body(
        base::reply("200 OK", &[], "{}"),
        Some(Duration::from_millis(200)),
    );
    let fixture = Fixture::new();
    let mut call = Call::new(serving.address);
    call.timeout = Some(Duration::from_secs(2));
    call.request_timeout = Duration::from_millis(50);
    let mut clients = clients();
    let mut logging = Logging::default();

    let outcome =
        async_create_interaction(&fixture, &mut clients, &mut logging, None, &call.request())
            .await
            .unwrap();

    let CreateOutcome::Response(body) = outcome else {
        panic!("a non-stream request returns the response");
    };
    assert_eq!(body, "{}");
    serving.request.join().unwrap();
}

#[tokio::test]
async fn async_create_interaction_leaves_a_response_transform_error_as_a_provider_error() {
    let serving = serve_with_body(base::reply("200 OK", &[], "{}"), None);
    let fixture = Fixture {
        fail_transform_status: Some(200),
        ..Fixture::new()
    };
    let mut clients = clients();
    let mut logging = Logging::default();

    let error = async_create_interaction(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        &Call::new(serving.address).request(),
    )
    .await
    .err()
    .unwrap();

    assert!(matches!(
        error,
        CreateInteractionError::Provider(FixtureError::Refused(200))
    ));
    assert!(
        fixture
            .calls()
            .iter()
            .all(|call| !call.starts_with("error|"))
    );
    serving.request.join().unwrap();
}

#[tokio::test]
async fn async_create_interaction_propagates_a_stream_iterator_error_without_the_error_class() {
    let serving = serve_with_body(base::reply("200 OK", &[], "{}"), None);
    let fixture = Fixture {
        refuse_stream: true,
        ..Fixture::new()
    };
    let mut call = Call::new(serving.address);
    call.stream = true;
    let mut clients = clients();
    let mut logging = Logging::default();

    let error =
        async_create_interaction(&fixture, &mut clients, &mut logging, None, &call.request())
            .await
            .err()
            .unwrap();

    assert!(matches!(
        error,
        CreateInteractionError::Provider(FixtureError::Stream)
    ));
    assert!(
        fixture
            .calls()
            .iter()
            .all(|call| !call.starts_with("error|"))
    );
    serving.request.join().unwrap();
}

#[tokio::test]
async fn async_create_interaction_refuses_a_proxy_it_cannot_use_instead_of_connecting_directly() {
    let fixture = Fixture::new();
    let mut clients = HttpClients::new(
        base::policy(Verification::Enabled, None),
        base::proxy_policy(&[("HTTP_PROXY", "https://127.0.0.1:1")]),
    );
    let mut logging = Logging::default();

    let error = async_create_interaction(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        &Call::new(dead_address()).request(),
    )
    .await
    .err()
    .unwrap();

    assert!(
        matches!(
            &error,
            CreateInteractionError::Transport(RequestFailure::UnsupportedProxyScheme { scheme })
                if scheme == "https"
        ),
        "{error:?}"
    );
}

#[tokio::test]
async fn async_create_interaction_skips_the_proxy_check_for_a_supplied_client() {
    let serving = serve_with_body(base::reply("200 OK", &[], "{}"), None);
    let fixture = Fixture::new();
    let supplied = base_builder().build().unwrap();
    let mut clients = HttpClients::new(
        base::policy(Verification::Enabled, None),
        base::proxy_policy(&[("HTTP_PROXY", "https://127.0.0.1:1")]),
    );
    let mut logging = Logging::default();

    let outcome = async_create_interaction(
        &fixture,
        &mut clients,
        &mut logging,
        Some(supplied),
        &Call::new(serving.address).request(),
    )
    .await
    .unwrap();

    let CreateOutcome::Response(body) = outcome else {
        panic!("a non-stream request returns the response");
    };
    assert_eq!(body, "{}");
    serving.request.join().unwrap();
}
