use std::time::Duration;

use betterlite_config::tls::Verification;
use betterlite_interactions::types::interaction::{InteractionInput, Turn, TurnContent};
use betterlite_models::controls::json_value::JsonValue;
use reqwest::blocking::ClientBuilder as BlockingClientBuilder;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::support::{
    Call, Fixture, FixtureError, api_key_header, caller_headers, client_error, dead_address,
    posted, provider_error, run, serve_with_body, transport_error,
};
use crate::interactions::base::clients::HttpClients;
use crate::interactions::base::tests::support as base;
use crate::interactions::base::{ClientError, Logging, Params, RequestFailure};
use crate::interactions::create::CreateOutcome;
use crate::interactions::create::synchronous::create_interaction;

#[test]
fn create_interaction_posts_the_provider_request_and_returns_the_transformed_response() {
    let serving = serve_with_body(base::reply("200 OK", &[], "{\"id\":\"int_1\"}"), None);
    let fixture = Fixture {
        headers: api_key_header(),
        data: base::params(r#"{"model": "m", "input": "hello"}"#),
        ..Fixture::new()
    };
    let mut call = Call::new(serving.address);
    call.agent = Some("agent_1".to_owned());
    call.optional_params = base::params(r#"{"store": false, "background": true}"#);
    call.extra_headers = caller_headers(&[("x-caller", "c1")]);
    call.params.insert(
        "api_base".to_owned(),
        JsonValue::Text(format!("http://{}/base", serving.address)),
    );

    let (outcome, logging) = run(&fixture, &call);

    assert_eq!(
        outcome.unwrap(),
        CreateOutcome::Response("{\"id\":\"int_1\"}".to_owned())
    );
    assert_eq!(
        fixture.calls(),
        vec![
            "environment|gemini-2.5-flash|1|1".to_owned(),
            format!(
                "url|http://{}/base|gemini-2.5-flash|agent_1|1|false",
                serving.address
            ),
            "create|gemini-2.5-flash|agent_1|true|2|1|1".to_owned(),
            "response|200".to_owned(),
            "model|gemini-2.5-flash".to_owned(),
        ]
    );
    let received = serving.request.join().unwrap();
    assert_eq!(
        received.head.lines().next(),
        Some("POST /base/v1beta/interactions HTTP/1.1")
    );
    assert!(received.head.contains("x-goog-api-key: key-1\r\n"));
    assert!(received.head.contains("content-type: application/json\r\n"));
    assert_eq!(
        posted(&received.body),
        posted(r#"{"model": "m", "input": "hello"}"#)
    );
    let recorded = logging.pre_call.as_ref().unwrap();
    assert_eq!(recorded.input, "hello");
    assert_eq!(recorded.api_key.expose_secret(), "");
    assert_eq!(
        recorded.api_base,
        format!("http://{}/base/v1beta/interactions", serving.address)
    );
    assert_eq!(recorded.headers["x-caller"].expose_secret(), "c1");
    assert_eq!(
        logging.model_call_details.get("complete_input_dict"),
        Some(&posted(r#"{"model": "m", "input": "hello"}"#))
    );
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

    let (outcome, logging) = run(&fixture, &call);

    assert_eq!(
        outcome.unwrap(),
        CreateOutcome::Stream(streaming.to_owned())
    );
    let calls = fixture.calls();
    assert!(
        calls.contains(&"sync_stream|200|gemini-2.5-flash".to_owned()),
        "{calls:?}"
    );
    assert_eq!(
        calls.get(1),
        Some(&format!(
            "url|http://{}|gemini-2.5-flash|-|1|true",
            serving.address
        ))
    );
    assert!(calls.iter().all(|call| !call.starts_with("response|")));
    assert_eq!(
        logging.model_call_details.get("streamed"),
        Some(&JsonValue::Bool(true))
    );
    serving.request.join().unwrap();
}

#[rstest]
#[case::non_streaming("404 Not Found", 404, false)]
#[case::streaming("500 Internal Server Error", 500, true)]
fn create_interaction_converts_a_failing_status_through_the_provider_error_class(
    #[case] status: &str,
    #[case] code: u16,
    #[case] stream: bool,
) {
    let serving = serve_with_body(base::reply(status, &[("retry-after", "5")], "boom"), None);
    let fixture = Fixture::new();
    let mut call = Call::new(serving.address);
    call.stream = stream;

    let (outcome, _logging) = run(&fixture, &call);

    assert_eq!(provider_error(outcome), FixtureError::Refused(code));
    let calls = fixture.calls();
    assert!(calls.contains(&format!("error|{code}|boom|5")), "{calls:?}");
    assert!(
        calls
            .iter()
            .all(|call| !call.starts_with("response|") && !call.starts_with("sync_stream|"))
    );
    serving.request.join().unwrap();
}

#[rstest]
#[case::transform_error(true, false, false, FixtureError::Refused(200))]
#[case::iterator_error(false, true, true, FixtureError::Stream)]
fn create_interaction_leaves_a_provider_error_from_the_config_unchanged(
    #[case] fail_transform: bool,
    #[case] refuse_stream: bool,
    #[case] stream: bool,
    #[case] expected: FixtureError,
) {
    let serving = serve_with_body(base::reply("200 OK", &[], "{}"), None);
    let fixture = Fixture {
        fail_transform_status: fail_transform.then_some(200),
        refuse_stream,
        ..Fixture::new()
    };
    let mut call = Call::new(serving.address);
    call.stream = stream;

    let (outcome, _logging) = run(&fixture, &call);

    assert_eq!(provider_error(outcome), expected);
    assert!(
        fixture
            .calls()
            .iter()
            .all(|call| !call.starts_with("error|"))
    );
    serving.request.join().unwrap();
}

#[rstest]
#[case::absent(None, "")]
#[case::structured(
    Some(InteractionInput::Turn(Turn {
        role: Some("user".to_owned()),
        content: Some(TurnContent::Text("hi".to_owned())),
    })),
    r#"{"role":"user","content":"hi"}"#
)]
fn create_interaction_logs_the_input_it_was_given(
    #[case] input: Option<InteractionInput>,
    #[case] expected: &str,
) {
    let serving = serve_with_body(base::reply("200 OK", &[], "{}"), None);
    let fixture = Fixture::new();
    let mut call = Call::new(serving.address);
    call.input = input;

    let (_outcome, logging) = run(&fixture, &call);

    assert_eq!(logging.pre_call.as_ref().unwrap().input, expected);
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

    let (_outcome, logging) = run(&fixture, &call);

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

    let (outcome, _logging) = run(&fixture, &call);

    assert!(matches!(
        client_error(outcome),
        ClientError::FieldType {
            field: "ssl_verify",
            ..
        }
    ));
    assert!(fixture.calls().is_empty());
}

#[rstest]
#[case::request_timeout(true)]
#[case::callers_timeout(false)]
fn create_interaction_uses_the_callers_timeout_else_the_request_timeout(#[case] expired: bool) {
    let serving = serve_with_body(
        base::reply("200 OK", &[], "{}"),
        Some(Duration::from_millis(200)),
    );
    let fixture = Fixture::new();
    let mut call = Call::new(serving.address);
    call.timeout = (!expired).then_some(Duration::from_secs(2));
    call.request_timeout = Duration::from_millis(50);

    let (outcome, _logging) = run(&fixture, &call);

    if expired {
        let failure = transport_error(outcome);
        assert!(matches!(failure, RequestFailure::Transport(source) if source.is_timeout()));
    } else {
        assert_eq!(outcome.unwrap(), CreateOutcome::Response("{}".to_owned()));
    }
    serving.request.join().unwrap();
}

#[test]
fn create_interaction_skips_the_proxy_check_for_a_supplied_client() {
    let serving = serve_with_body(base::reply("200 OK", &[], "{}"), None);
    let fixture = Fixture::new();
    let mut clients = HttpClients::new(
        base::policy(Verification::Enabled, None),
        base::proxy_policy(&[("HTTP_PROXY", "https://127.0.0.1:1")]),
    );

    let outcome = create_interaction(
        &fixture,
        &mut clients,
        &mut Logging::default(),
        Some(BlockingClientBuilder::new().build().unwrap()),
        &Call::new(serving.address).request(),
    );

    assert_eq!(outcome.unwrap(), CreateOutcome::Response("{}".to_owned()));
    serving.request.join().unwrap();
}
