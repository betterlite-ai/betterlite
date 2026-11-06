use super::{FakeTransport, HttpReply, Outcome, TransportFailure, run};

const VALIDATING: FakeTransport = FakeTransport {
    validating: true,
    outcome: Outcome::Echo,
};
const TIMING_OUT: FakeTransport = FakeTransport {
    validating: false,
    outcome: Outcome::Fail(TransportFailure::TimedOut),
};

fn reply(status: u16, body: &str) -> Outcome {
    let headers = vec![("x-reply".to_owned(), "yes".to_owned())];
    Outcome::Reply(HttpReply {
        status,
        headers,
        body: body.to_owned(),
    })
}

#[rstest::rstest]
#[case::get_without_options(FakeTransport::default(), "http_get", r#"["https://example.com/x"]"#, "{}",
    r#""via":"safe_get","method":"GET","url":"https://example.com/x","headers":{},"body":null,"timeout_ms":30000"#)]
#[case::get_strips_host_while_validating(
    VALIDATING,
    "http_get",
    r#"["https://example.com/x",{"Host":"evil","X-A":"1"}]"#,
    "{}",
    r#""headers":{"X-A":"1"}"#
)]
#[case::get_keeps_host_without_validation(
    FakeTransport::default(),
    "http_get",
    r#"["https://example.com/x",{"Host":"evil","X-A":"1"}]"#,
    "{}",
    r#""headers":{"Host":"evil","X-A":"1"}"#
)]
#[case::post_threads_the_validated_destination(VALIDATING, "http_post", r#"["https://example.com/p",{"a":1},{"X-A":"1","host":"evil"}]"#, "{}",
    r#""via":"send","method":"POST","url":"https://93.184.216.34/p","headers":{"X-A":"1","Host":"example.com"},"body":{"a":1}"#)]
#[case::post_keeps_host_without_validation(
    FakeTransport::default(),
    "http_post",
    r#"["https://example.com/p",{"a":1},{"Host":"custom"}]"#,
    "{}",
    r#""url":"https://example.com/p","headers":{"Host":"custom"},"body":{"a":1}"#
)]
#[case::post_sends_a_string_body_as_text(
    FakeTransport::default(),
    "http_post",
    r#"["https://example.com/p","raw"]"#,
    "{}",
    r#""body":"raw""#
)]
#[case::post_sends_a_list_body_as_json_text(
    FakeTransport::default(),
    "http_post",
    r#"["https://example.com/p",[1,2]]"#,
    "{}",
    r#""body":"[1,2]""#
)]
#[case::post_without_a_body(
    FakeTransport::default(),
    "http_post",
    r#"["https://example.com/p"]"#,
    "{}",
    r#""method":"POST","url":"https://example.com/p","headers":{},"body":null"#
)]
#[case::request_defaults_to_get(
    FakeTransport::default(),
    "http_request",
    r#"["https://example.com/x"]"#,
    "{}",
    r#""via":"safe_get","method":"GET""#
)]
#[case::request_upper_cases_the_method(
    FakeTransport::default(),
    "http_request",
    r#"["https://example.com/x"]"#,
    r#"{"method":"put","body":"x"}"#,
    r#""via":"send","method":"PUT""#
)]
#[case::request_delete(
    FakeTransport::default(),
    "http_request",
    r#"["https://example.com/x","DELETE"]"#,
    "{}",
    r#""via":"send","method":"DELETE""#
)]
#[case::request_patch(
    FakeTransport::default(),
    "http_request",
    r#"["https://example.com/x","Patch"]"#,
    "{}",
    r#""via":"send","method":"PATCH""#
)]
#[case::timeout_below_the_minimum_is_raised(
    FakeTransport::default(),
    "http_get",
    r#"["https://example.com/x",null,0.01]"#,
    "{}",
    r#""timeout_ms":100"#
)]
#[case::timeout_above_the_maximum_is_lowered(
    FakeTransport::default(),
    "http_get",
    r#"["https://example.com/x",null,500]"#,
    "{}",
    r#""timeout_ms":60000"#
)]
#[case::timeout_in_range_is_kept(
    FakeTransport::default(),
    "http_get",
    r#"["https://example.com/x"]"#,
    r#"{"timeout":5}"#,
    r#""timeout_ms":5000"#
)]
#[case::invalid_method(FakeTransport::default(), "http_request", r#"["https://example.com/x","TRACE"]"#, "{}",
    r#"{"status_code":0,"body":null,"headers":{},"success":false,"error":"Invalid HTTP method: TRACE. Allowed: GET, POST, PUT, DELETE, PATCH"}"#)]
#[case::invalid_url(FakeTransport::default(), "http_get", r#"["not a url"]"#, "{}",
    r#"{"status_code":0,"body":null,"headers":{},"success":false,"error":"Invalid URL: not a url"}"#)]
#[case::json_reply_is_parsed(FakeTransport { validating: false, outcome: reply(200, r#"{"ok":true}"#) }, "http_get", r#"["https://example.com/x"]"#, "{}",
    r#"{"status_code":200,"body":{"ok":true},"headers":{"x-reply":"yes"},"success":true,"error":null}"#)]
#[case::text_reply_stays_text(FakeTransport { validating: false, outcome: reply(404, "not json") }, "http_get", r#"["https://example.com/x"]"#, "{}",
    r#"{"status_code":404,"body":"not json","headers":{"x-reply":"yes"},"success":false,"error":null}"#)]
#[case::status_299_succeeds(FakeTransport { validating: false, outcome: reply(299, "") }, "http_get", r#"["https://example.com/x"]"#, "{}",
    r#""status_code":299,"body":"","headers":{"x-reply":"yes"},"success":true"#)]
#[case::status_300_fails(FakeTransport { validating: false, outcome: reply(300, "") }, "http_get", r#"["https://example.com/x"]"#, "{}",
    r#""status_code":300,"body":"","headers":{"x-reply":"yes"},"success":false"#)]
#[case::repeated_headers_are_lowercased_and_joined(
    FakeTransport { validating: false, outcome: Outcome::Reply(HttpReply {
        status: 200,
        headers: vec![("Set-Cookie".to_owned(), "a=1".to_owned()), ("set-cookie".to_owned(), "b=2".to_owned())],
        body: String::new(),
    }) },
    "http_get",
    r#"["https://example.com/x"]"#,
    "{}",
    r#""headers":{"set-cookie":"a=1, b=2"}"#
)]
#[case::get_validation_turns_redirects_off(
    VALIDATING,
    "http_get",
    r#"["https://example.com/x"]"#,
    "{}",
    r#""connect_ms":5000,"redirects":false"#
)]
#[case::get_without_validation_follows_redirects(
    FakeTransport::default(),
    "http_get",
    r#"["https://example.com/x"]"#,
    "{}",
    r#""connect_ms":5000,"redirects":true"#
)]
#[case::post_validation_turns_redirects_off(
    VALIDATING,
    "http_post",
    r#"["https://example.com/p"]"#,
    "{}",
    r#""connect_ms":5000,"redirects":false"#
)]
#[case::refused_destination(FakeTransport { validating: false, outcome: Outcome::Fail(TransportFailure::Refused { message: "private address".to_owned() }) }, "http_get", r#"["https://example.com/x"]"#, "{}",
    r#""success":false,"error":"Blocked URL: private address""#)]
#[case::refused_by_validation_before_a_post(FakeTransport { validating: true, outcome: Outcome::ValidationRefused(TransportFailure::Refused { message: "metadata address".to_owned() }) }, "http_post",
    r#"["https://example.com/p"]"#, "{}", r#""error":"Blocked URL: metadata address""#)]
#[case::timeout_names_the_default(
    TIMING_OUT,
    "http_get",
    r#"["https://example.com/x"]"#,
    "{}",
    r#""error":"Request timeout after 30.0s""#
)]
#[case::timeout_names_the_clamped_minimum(
    TIMING_OUT,
    "http_get",
    r#"["https://example.com/x",null,0.01]"#,
    "{}",
    r#""error":"Request timeout after 0.1s""#
)]
#[case::timeout_names_the_clamped_maximum(
    TIMING_OUT,
    "http_get",
    r#"["https://example.com/x",null,500]"#,
    "{}",
    r#""error":"Request timeout after 60.0s""#
)]
#[case::timeout_names_a_fraction(
    TIMING_OUT,
    "http_get",
    r#"["https://example.com/x",null,2.5]"#,
    "{}",
    r#""error":"Request timeout after 2.5s""#
)]
#[case::transport_failure(FakeTransport { validating: false, outcome: Outcome::Fail(TransportFailure::Failed { message: "connection reset".to_owned() }) }, "http_get", r#"["https://example.com/x"]"#, "{}",
    r#""error":"Request failed: connection reset""#)]
#[tokio::test]
async fn primitives_custom_code_async(
    #[case] transport: FakeTransport,
    #[case] name: &str,
    #[case] positional: &str,
    #[case] named: &str,
    #[case] expected: &str,
) {
    let output = run(transport, name, positional, named)
        .await
        .expect("the call succeeds");
    assert!(output.contains(expected), "{expected} is not in {output}");
}
