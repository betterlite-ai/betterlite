use std::time::Duration;

use betterlite_config::tls::Verification;
use betterlite_http_fixture::server::serve;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::identity::provider_id::ProviderId;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::support::{
    Call, Fixture, FixtureError, api_key_header, clients, dead_address, posted, provider_error,
    run_async, transport_error,
};
use crate::interactions::base::clients::HttpClients;
use crate::interactions::base::tests::support as base;
use crate::interactions::base::{Logging, Params, RequestFailure};
use crate::interactions::create::CreateOutcome;
use crate::interactions::create::asynchronous::async_create_interaction;
use crate::interactions::streaming::AsyncStreamingConfig;
use crate::proxy::StreamResponse;
use crate::proxy::base_builder;

// the async half of the fixture lives here, apart from the sync half in support
impl AsyncStreamingConfig for Fixture {
    type AsyncStream = StreamResponse;

    fn create_async_stream_iterator(
        &self,
        response: StreamResponse,
        model: Option<&str>,
        logging: &mut Logging,
    ) -> Result<StreamResponse, FixtureError> {
        let status_code = response.response.status().as_u16();
        self.note_stream("async_stream", status_code, model, logging)?;
        Ok(response)
    }
}

#[tokio::test]
async fn async_create_interaction_posts_the_provider_request_and_returns_the_response() {
    let serving = serve(
        base::reply("200 OK", &[("x-request-id", "r1")], "{\"id\":\"int_1\"}"),
        Duration::ZERO,
    )
    .unwrap();
    let fixture = Fixture {
        headers: api_key_header(),
        data: base::params(r#"{"model": "m", "input": "hello"}"#),
        ..Fixture::new()
    };

    let mut call = Call::new(serving.address);
    call.extra_body = Some(Params::from([("store".to_owned(), JsonValue::Bool(true))]));

    let (outcome, logging) = run_async(&fixture, &call).await;

    let CreateOutcome::Response(body) = outcome.unwrap() else {
        panic!("a non-streaming request returns the response");
    };
    assert_eq!(body, "{\"id\":\"int_1\"}");
    let expected = posted(r#"{"input": "hello", "model": "m", "store": true}"#);
    let address = serving.address;
    let received = serving.finish().unwrap();
    assert_eq!(
        received.head.lines().next(),
        Some("POST /v1beta/interactions HTTP/1.1")
    );
    assert!(received.head.contains("x-goog-api-key: key-1\r\n"));
    assert_eq!(posted(&received.body), expected);
    assert_eq!(
        logging.model_call_details.get("complete_input_dict"),
        Some(&expected),
        "the logged body carries the overlay"
    );
    let recorded = logging.pre_call.as_ref().unwrap();
    assert_eq!(recorded.input, "hello");
    assert_eq!(recorded.api_key.expose_secret(), "");
    assert_eq!(
        recorded.api_base,
        format!("http://{}/v1beta/interactions", address)
    );
    assert!(fixture.calls().contains(&"response|200|r1".to_owned()));
}

#[tokio::test]
async fn async_create_interaction_returns_the_open_response_as_a_stream_iterator() {
    let streaming = "data: {\"event_type\": \"step.start\"}\n\ndata: [DONE]\n\n";
    let serving = serve(base::reply("200 OK", &[], streaming), Duration::ZERO).unwrap();
    let fixture = Fixture {
        headers: api_key_header(),
        ..Fixture::new()
    };
    let mut call = Call::new(serving.address);
    call.stream = true;

    let (outcome, logging) = run_async(&fixture, &call).await;

    let CreateOutcome::Stream(source) = outcome.unwrap() else {
        panic!("a stream request returns the iterator");
    };
    // the iterator receives the body unread, so this reads what its first step would have read
    assert_eq!(source.response.text().await.unwrap(), streaming);
    let calls = fixture.calls();
    assert!(
        calls.contains(&"async_stream|200|gemini-2.5-flash".to_owned()),
        "{calls:?}"
    );
    assert!(calls.iter().all(|entry| !entry.starts_with("response|")));
    assert_eq!(
        logging.model_call_details.get("streamed"),
        Some(&JsonValue::Bool(true))
    );
    serving.finish().unwrap();
}

#[rstest]
#[case::non_streaming("404 Not Found", 404, false)]
#[case::streaming("500 Internal Server Error", 500, true)]
#[tokio::test]
async fn async_create_interaction_converts_a_failing_status_through_the_provider_error_class(
    #[case] status: &str,
    #[case] code: u16,
    #[case] stream: bool,
) {
    let serving = serve(
        base::reply(status, &[("retry-after", "5")], "boom"),
        Duration::ZERO,
    )
    .unwrap();
    let fixture = Fixture::new();
    let mut call = Call::new(serving.address);
    call.stream = stream;

    let (outcome, _logging) = run_async(&fixture, &call).await;

    assert_eq!(provider_error(outcome), FixtureError::Refused(code));
    let calls = fixture.calls();
    assert!(calls.contains(&format!("error|{code}|boom|5")), "{calls:?}");
    assert!(
        calls
            .iter()
            .all(|entry| !entry.starts_with("response|") && !entry.starts_with("async_stream|"))
    );
    serving.finish().unwrap();
}

#[rstest]
#[case::request_timeout(true)]
#[case::callers_timeout(false)]
#[tokio::test]
async fn async_create_interaction_uses_the_callers_timeout_else_the_request_timeout(
    #[case] expired: bool,
) {
    let serving = serve(base::reply("200 OK", &[], "{}"), Duration::from_millis(200)).unwrap();
    let fixture = Fixture::new();
    let mut call = Call::new(serving.address);
    call.timeout = (!expired).then_some(Duration::from_secs(2));
    call.request_timeout = Duration::from_millis(50);

    let (outcome, _logging) = run_async(&fixture, &call).await;

    if expired {
        let failure = transport_error(outcome);
        assert!(matches!(failure, RequestFailure::Transport(source) if source.is_timeout()));
    } else {
        let CreateOutcome::Response(body) = outcome.unwrap() else {
            panic!("a non-streaming request returns the response");
        };
        assert_eq!(body, "{}");
    }
    serving.finish().unwrap();
}

#[tokio::test]
async fn async_create_interaction_returns_a_connection_failure_to_the_caller() {
    let fixture = Fixture::new();

    let (outcome, _logging) = run_async(&fixture, &Call::new(dead_address())).await;

    assert!(matches!(
        transport_error(outcome),
        RequestFailure::Transport(source) if source.is_connect()
    ));
}

#[tokio::test]
async fn async_create_interaction_leaves_a_config_error_as_a_provider_error() {
    let serving = serve(base::reply("200 OK", &[], "{}"), Duration::ZERO).unwrap();
    let fixture = Fixture {
        fail_transform_status: Some(200),
        ..Fixture::new()
    };

    let (outcome, _logging) = run_async(&fixture, &Call::new(serving.address)).await;

    assert_eq!(provider_error(outcome), FixtureError::Refused(200));
    assert!(
        fixture
            .calls()
            .iter()
            .all(|recorded| !recorded.starts_with("error|"))
    );
    serving.finish().unwrap();
}

#[tokio::test]
async fn async_create_interaction_does_not_follow_a_redirect() {
    let serving = serve(
        base::reply(
            "302 Found",
            &[("location", "http://127.0.0.1:1/elsewhere")],
            "",
        ),
        Duration::ZERO,
    )
    .unwrap();
    let fixture = Fixture::new();

    let (outcome, _logging) = run_async(&fixture, &Call::new(serving.address)).await;

    assert_eq!(provider_error(outcome), FixtureError::Refused(302));
    let calls = fixture.calls();
    assert!(calls.contains(&"error|302||-".to_owned()), "{calls:?}");
    serving.finish().unwrap();
}

#[tokio::test]
async fn async_create_interaction_passes_an_invalid_url_through_the_error_handler() {
    let fixture = Fixture::new();
    let mut call = Call::new(dead_address());
    call.params.insert(
        "api_base".to_owned(),
        JsonValue::Text("not a url".to_owned()),
    );

    let (outcome, _logging) = run_async(&fixture, &call).await;

    assert!(matches!(transport_error(outcome), RequestFailure::Url(_)));
    assert!(
        fixture
            .calls()
            .iter()
            .all(|recorded| !recorded.starts_with("error|"))
    );
}

#[tokio::test]
async fn async_create_interaction_refuses_a_proxy_it_cannot_use_instead_of_connecting_directly() {
    let fixture = Fixture::new();
    let mut clients = HttpClients::new(
        base::policy(Verification::Enabled, None),
        base::proxy_policy(&[("HTTP_PROXY", "https://127.0.0.1:1")]),
    );

    let outcome = async_create_interaction(
        &fixture,
        &mut clients,
        &mut Logging::default(),
        None,
        &Call::new(dead_address()).request(),
    )
    .await;

    assert!(matches!(
        transport_error(outcome),
        RequestFailure::UnsupportedProxyScheme { scheme } if scheme == "https"
    ));
}

#[tokio::test]
async fn async_create_interaction_builds_for_the_provider_its_caller_names() {
    let serving = serve(base::reply("200 OK", &[], "{}"), Duration::ZERO).unwrap();
    let fixture = Fixture::new();
    let mut clients = clients();
    let mut call = Call::new(serving.address);
    call.provider = ProviderId::Openai;

    let outcome = async_create_interaction(
        &fixture,
        &mut clients,
        &mut Logging::default(),
        None,
        &call.request(),
    )
    .await
    .unwrap();

    let CreateOutcome::Response(body) = outcome else {
        panic!("a non-streaming request returns the response");
    };
    assert_eq!(body, "{}");
    let providers: Vec<ProviderId> = clients
        .async_clients
        .keys()
        .map(|key| key.provider)
        .collect();
    assert!(providers.contains(&ProviderId::Openai));
    assert!(!providers.contains(&ProviderId::Gemini));
    serving.finish().unwrap();
}

#[tokio::test]
async fn async_create_interaction_uses_a_supplied_client_as_is() {
    let serving = serve(base::reply("200 OK", &[], "{}"), Duration::ZERO).unwrap();
    let fixture = Fixture::new();
    let mut clients = HttpClients::new(
        base::policy(Verification::Enabled, None),
        base::proxy_policy(&[("HTTP_PROXY", "https://127.0.0.1:1")]),
    );
    let supplied = base_builder().build().unwrap();

    let outcome = async_create_interaction(
        &fixture,
        &mut clients,
        &mut Logging::default(),
        Some(supplied),
        &Call::new(serving.address).request(),
    )
    .await
    .unwrap();

    let CreateOutcome::Response(body) = outcome else {
        panic!("a non-streaming request returns the response");
    };
    assert_eq!(body, "{}");
    assert!(clients.async_clients.is_empty());
    serving.finish().unwrap();
}
