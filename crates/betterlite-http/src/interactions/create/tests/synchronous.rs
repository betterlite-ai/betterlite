use std::time::Duration;

use betterlite_config::tls::Verification;
use betterlite_interactions::types::interaction::{InteractionInput, Turn, TurnContent};
use betterlite_models::controls::json_value::JsonValue;
use reqwest::blocking::ClientBuilder as BlockingClientBuilder;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::support::{
    Call, Fixture, FixtureError, api_key_header, caller_headers, clients, dead_address,
    serve_with_body,
};
use crate::interactions::base::clients::HttpClients;
use crate::interactions::base::tests::support as base;
use crate::interactions::base::{ClientError, Logging, Params, RequestFailure};
use crate::interactions::create::synchronous::create_interaction;
use crate::interactions::create::{CreateInteractionError, CreateOutcome};

fn posted(body: &str) -> JsonValue {
    serde_json::from_str(body).unwrap()
}

#[test]
fn create_interaction_posts_the_transformed_request_and_returns_the_transformed_response() {
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
        create_interaction(&fixture, &mut clients, &mut logging, None, &call.request()).unwrap();

    assert_eq!(
        outcome,
        CreateOutcome::Response("{\"id\":\"int_1\"}".to_owned())
    );
    let received = serving.request.join().unwrap();
    assert!(
        received
            .head
            .starts_with("POST /v1beta/interactions HTTP/1.1"),
        "{}",
        received.head
    );
    assert!(received.head.contains("x-goog-api-key: key-1\r\n"));
    assert!(received.head.contains("content-type: application/json\r\n"));
    assert_eq!(
        posted(&received.body),
        posted(r#"{"model": "gemini-2.5-flash", "input": "hello"}"#)
    );
}

#[test]
fn create_interaction_calls_the_provider_contract_before_it_logs_the_response() {
    let serving = serve_with_body(base::reply("200 OK", &[("x-request-id", "r1")], "{}"), None);
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
    call.extra_headers = caller_headers(&[("x-caller", "c1")]);
    let mut clients = clients();
    let mut logging = Logging::default();

    create_interaction(&fixture, &mut clients, &mut logging, None, &call.request()).unwrap();

    assert_eq!(
        fixture.calls(),
        vec![
            "environment|gemini-2.5-flash|1|2".to_owned(),
            format!("url|http://{}|gemini-2.5-flash|-|2|false", serving.address),
            "create|gemini-2.5-flash|-|true|0|2|1".to_owned(),
            "response|200|hello|r1".to_owned(),
        ]
    );
    assert_eq!(
        logging.model_call_details.get("model"),
        Some(&JsonValue::Text("gemini-2.5-flash".to_owned()))
    );
    serving.request.join().unwrap();
}

#[rstest]
#[case::absent(&[], "environment|gemini-2.5-flash|0|1")]
#[case::given(&[("x-caller", "c1")], "environment|gemini-2.5-flash|1|1")]
fn create_interaction_sends_the_caller_headers_to_validate_environment(
    #[case] extra: &[(&str, &str)],
    #[case] expected: &str,
) {
    let serving = serve_with_body(base::reply("200 OK", &[], "{}"), None);
    let fixture = Fixture::new();
    let mut call = Call::new(serving.address);
    call.extra_headers = caller_headers(extra);
    let mut clients = clients();
    let mut logging = Logging::default();

    create_interaction(&fixture, &mut clients, &mut logging, None, &call.request()).unwrap();

    assert_eq!(fixture.calls().first().map(String::as_str), Some(expected));
    serving.request.join().unwrap();
}

#[test]
fn create_interaction_takes_the_api_base_from_litellm_params() {
    let serving = serve_with_body(base::reply("200 OK", &[], "{}"), None);
    let fixture = Fixture::new();
    let mut call = Call::new(serving.address);
    call.params.insert(
        "api_base".to_owned(),
        JsonValue::Text(format!("http://{}/base", serving.address)),
    );
    let mut clients = clients();
    let mut logging = Logging::default();

    let outcome =
        create_interaction(&fixture, &mut clients, &mut logging, None, &call.request()).unwrap();

    assert_eq!(outcome, CreateOutcome::Response("{}".to_owned()));
    let received = serving.request.join().unwrap();
    assert!(
        received
            .head
            .starts_with("POST /base/v1beta/interactions HTTP/1.1"),
        "{}",
        received.head
    );
    let pre_call = logging.pre_call.as_ref().unwrap();
    assert_eq!(
        pre_call.api_base,
        format!("http://{}/base/v1beta/interactions", serving.address)
    );
}

#[rstest]
#[case::non_streaming(false)]
#[case::streaming(true)]
fn create_interaction_passes_the_stream_flag_to_the_url_builder(#[case] stream: bool) {
    let serving = serve_with_body(base::reply("200 OK", &[], "{}"), None);
    let fixture = Fixture::new();
    let mut call = Call::new(serving.address);
    call.stream = stream;
    call.agent = Some("agent_1".to_owned());
    let mut clients = clients();
    let mut logging = Logging::default();

    create_interaction(&fixture, &mut clients, &mut logging, None, &call.request()).unwrap();

    let expected = format!(
        "url|http://{}|gemini-2.5-flash|agent_1|1|{stream}",
        serving.address
    );
    assert_eq!(fixture.calls().get(1), Some(&expected));
    serving.request.join().unwrap();
}

#[test]
fn create_interaction_passes_every_parameter_to_the_transform() {
    let serving = serve_with_body(base::reply("200 OK", &[], "{}"), None);
    let fixture = Fixture {
        headers: api_key_header(),
        ..Fixture::new()
    };
    let mut call = Call::new(serving.address);
    call.agent = Some("agent_1".to_owned());
    call.optional_params = base::params(r#"{"store": false, "background": true}"#);
    call.params.insert(
        "custom_llm_provider".to_owned(),
        JsonValue::Text("gemini".to_owned()),
    );
    let mut clients = clients();
    let mut logging = Logging::default();

    create_interaction(&fixture, &mut clients, &mut logging, None, &call.request()).unwrap();

    assert_eq!(
        fixture.calls().get(2).map(String::as_str),
        Some("create|gemini-2.5-flash|agent_1|true|2|2|1")
    );
    serving.request.join().unwrap();
}

#[test]
fn create_interaction_returns_a_stream_iterator_for_a_stream_request() {
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
        create_interaction(&fixture, &mut clients, &mut logging, None, &call.request()).unwrap();

    assert_eq!(outcome, CreateOutcome::Stream(streaming.to_owned()));
    let calls = fixture.calls();
    let opened = format!("sync_stream|200|gemini-2.5-flash|{streaming}");
    assert!(calls.contains(&opened), "{calls:?}");
    assert!(calls.iter().all(|call| !call.starts_with("response|")));
    assert_eq!(
        logging.model_call_details.get("streamed"),
        Some(&JsonValue::Bool(true))
    );
    serving.request.join().unwrap();
}

#[test]
fn create_interaction_converts_a_failing_status_through_the_provider_error_class() {
    let serving = serve_with_body(
        base::reply("404 Not Found", &[("retry-after", "5")], "missing"),
        None,
    );
    let fixture = Fixture::new();
    let mut clients = clients();
    let mut logging = Logging::default();

    let error = create_interaction(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        &Call::new(serving.address).request(),
    )
    .unwrap_err();

    assert!(matches!(
        error,
        CreateInteractionError::Provider(FixtureError::Refused(404))
    ));
    let calls = fixture.calls();
    assert!(
        calls.iter().any(|call| call == "error|404|missing|5"),
        "{calls:?}"
    );
    assert!(calls.iter().all(|call| !call.starts_with("response|")));
    serving.request.join().unwrap();
}

#[test]
fn create_interaction_converts_a_failing_status_for_a_stream_request() {
    let serving = serve_with_body(
        base::reply("500 Internal Server Error", &[("retry-after", "5")], "boom"),
        None,
    );
    let fixture = Fixture::new();
    let mut call = Call::new(serving.address);
    call.stream = true;
    let mut clients = clients();
    let mut logging = Logging::default();

    let error = create_interaction(&fixture, &mut clients, &mut logging, None, &call.request())
        .unwrap_err();

    assert!(matches!(
        error,
        CreateInteractionError::Provider(FixtureError::Refused(500))
    ));
    let calls = fixture.calls();
    assert!(
        calls.iter().any(|call| call == "error|500|boom|5"),
        "{calls:?}"
    );
    assert!(calls.iter().all(|call| !call.starts_with("sync_stream|")));
    serving.request.join().unwrap();
}

#[test]
fn create_interaction_leaves_a_response_transform_error_as_a_provider_error() {
    let serving = serve_with_body(base::reply("200 OK", &[], "{}"), None);
    let fixture = Fixture {
        fail_transform_status: Some(200),
        ..Fixture::new()
    };
    let mut clients = clients();
    let mut logging = Logging::default();

    let error = create_interaction(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        &Call::new(serving.address).request(),
    )
    .unwrap_err();

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

#[test]
fn create_interaction_propagates_a_stream_iterator_error_without_the_error_class() {
    let serving = serve_with_body(base::reply("200 OK", &[], "{}"), None);
    let fixture = Fixture {
        refuse_stream: true,
        ..Fixture::new()
    };
    let mut call = Call::new(serving.address);
    call.stream = true;
    let mut clients = clients();
    let mut logging = Logging::default();

    let error = create_interaction(&fixture, &mut clients, &mut logging, None, &call.request())
        .unwrap_err();

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

#[test]
fn create_interaction_logs_the_pre_call_with_the_input_and_the_complete_input_dict() {
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

    create_interaction(&fixture, &mut clients, &mut logging, None, &call.request()).unwrap();

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

#[test]
fn create_interaction_logs_a_structured_input_as_json() {
    let serving = serve_with_body(base::reply("200 OK", &[], "{}"), None);
    let fixture = Fixture::new();
    let mut call = Call::new(serving.address);
    call.input = Some(InteractionInput::Turn(Turn {
        role: Some("user".to_owned()),
        content: Some(TurnContent::Text("hi".to_owned())),
    }));
    let mut clients = clients();
    let mut logging = Logging::default();

    create_interaction(&fixture, &mut clients, &mut logging, None, &call.request()).unwrap();

    let recorded = logging.pre_call.as_ref().unwrap();
    assert_eq!(
        posted(&recorded.input),
        posted(r#"{"role": "user", "content": "hi"}"#)
    );
    serving.request.join().unwrap();
}

#[test]
fn create_interaction_logs_an_absent_input_as_empty() {
    let serving = serve_with_body(base::reply("200 OK", &[], "{}"), None);
    let fixture = Fixture::new();
    let mut call = Call::new(serving.address);
    call.input = None;
    let mut clients = clients();
    let mut logging = Logging::default();

    create_interaction(&fixture, &mut clients, &mut logging, None, &call.request()).unwrap();

    assert_eq!(logging.pre_call.as_ref().unwrap().input, "");
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
fn create_interaction_overlays_extra_body_onto_the_transformed_request(
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

    create_interaction(&fixture, &mut clients, &mut logging, None, &call.request()).unwrap();

    let received = serving.request.join().unwrap();
    assert_eq!(posted(&received.body), posted(expected));
    assert_eq!(
        logging.model_call_details.get("complete_input_dict"),
        Some(&posted(expected)),
        "the logged body carries the overlay"
    );
}

#[test]
fn create_interaction_builds_the_client_before_validating_headers() {
    let fixture = Fixture::new();
    let mut call = Call::new(dead_address());
    call.params
        .insert("ssl_verify".to_owned(), JsonValue::Text("yes".to_owned()));
    let mut clients = clients();
    let mut logging = Logging::default();

    let error = create_interaction(&fixture, &mut clients, &mut logging, None, &call.request())
        .unwrap_err();

    assert!(matches!(
        error,
        CreateInteractionError::Client(ClientError::FieldType {
            field: "ssl_verify",
            ..
        })
    ));
    assert!(fixture.calls().is_empty());
}

#[test]
fn create_interaction_reports_a_transport_failure_without_the_error_class() {
    let fixture = Fixture::new();
    let mut clients = clients();
    let mut logging = Logging::default();

    let error = create_interaction(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        &Call::new(dead_address()).request(),
    )
    .unwrap_err();

    assert!(matches!(
        error,
        CreateInteractionError::Transport(RequestFailure::Transport(_))
    ));
    assert!(
        fixture
            .calls()
            .iter()
            .all(|call| !call.starts_with("error|"))
    );
}

#[test]
fn create_interaction_does_not_follow_a_redirect() {
    let serving = serve_with_body(
        base::reply("302 Found", &[("location", "http://127.0.0.1:1/x")], ""),
        None,
    );
    let fixture = Fixture::new();
    let mut clients = clients();
    let mut logging = Logging::default();

    let error = create_interaction(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        &Call::new(serving.address).request(),
    )
    .unwrap_err();

    assert!(matches!(
        error,
        CreateInteractionError::Provider(FixtureError::Refused(302))
    ));
    serving.request.join().unwrap();
}

#[test]
fn create_interaction_times_out_on_the_request_timeout_when_the_caller_names_none() {
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

    let error = create_interaction(&fixture, &mut clients, &mut logging, None, &call.request())
        .unwrap_err();

    assert!(matches!(
        error,
        CreateInteractionError::Transport(RequestFailure::Transport(source)) if source.is_timeout()
    ));
    serving.request.join().unwrap();
}

#[test]
fn create_interaction_prefers_the_callers_timeout() {
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
        create_interaction(&fixture, &mut clients, &mut logging, None, &call.request()).unwrap();

    assert_eq!(outcome, CreateOutcome::Response("{}".to_owned()));
    serving.request.join().unwrap();
}

#[test]
fn create_interaction_refuses_a_proxy_it_cannot_use_instead_of_connecting_directly() {
    let fixture = Fixture::new();
    let mut clients = HttpClients::new(
        base::policy(Verification::Enabled, None),
        base::proxy_policy(&[("HTTP_PROXY", "https://127.0.0.1:1")]),
    );
    let mut logging = Logging::default();

    let error = create_interaction(
        &fixture,
        &mut clients,
        &mut logging,
        None,
        &Call::new(dead_address()).request(),
    )
    .unwrap_err();

    assert!(
        matches!(
            &error,
            CreateInteractionError::Transport(RequestFailure::UnsupportedProxyScheme { scheme })
                if scheme == "https"
        ),
        "{error:?}"
    );
}

#[test]
fn create_interaction_skips_the_proxy_check_for_a_supplied_client() {
    let serving = serve_with_body(base::reply("200 OK", &[], "{}"), None);
    let fixture = Fixture::new();
    let supplied = BlockingClientBuilder::new().build().unwrap();
    let mut clients = HttpClients::new(
        base::policy(Verification::Enabled, None),
        base::proxy_policy(&[("HTTP_PROXY", "https://127.0.0.1:1")]),
    );
    let mut logging = Logging::default();

    let outcome = create_interaction(
        &fixture,
        &mut clients,
        &mut logging,
        Some(supplied),
        &Call::new(serving.address).request(),
    )
    .unwrap();

    assert_eq!(outcome, CreateOutcome::Response("{}".to_owned()));
    serving.request.join().unwrap();
}
