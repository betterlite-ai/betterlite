#![cfg(test)]

use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use betterlite_http::user_url::httpx_url::HttpxUrl;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use futures_util::Stream;
use futures_util::stream;
use http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::time::Instant;

use super::secrets::{
    header_secret_values, mask_known_values, request_secret_values, sensitive_field,
};
use super::text::{safe_text, safe_upstream_url};
use super::upstream::{
    BodyState, UpstreamFailure, UpstreamRequest, UpstreamResponse, capture_error_response,
};
use super::{
    CAPTURE_FAILED, CREDENTIALS_UNAVAILABLE, UNSTRUCTURED_BODY, describe_upstream_http_failure,
    describe_upstream_response, masked_headers, preview, request_body_preview,
    response_body_preview,
};

fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("built-in patterns compile")
}

fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in pairs {
        map.append(
            HeaderName::from_bytes(name.as_bytes()).expect("header name"),
            HeaderValue::from_str(value).expect("header value"),
        );
    }
    map
}

fn url(text: &str) -> HttpxUrl {
    HttpxUrl::parse(text).expect("valid url")
}

fn exposed(values: &[SecretString]) -> Vec<&str> {
    values.iter().map(SecretString::expose_secret).collect()
}

fn request(
    method: &str,
    address: &str,
    pairs: &[(&str, &str)],
    body: BodyState,
) -> UpstreamRequest {
    UpstreamRequest {
        method: String::from(method),
        url: url(address),
        headers: headers(pairs),
        body,
    }
}

fn response(
    status: u16,
    pairs: &[(&str, &str)],
    body: BodyState,
    source: Option<UpstreamRequest>,
) -> UpstreamResponse {
    UpstreamResponse {
        status: StatusCode::from_u16(status).expect("status"),
        headers: headers(pairs),
        body,
        request: source,
        captured: None,
    }
}

fn json_request() -> UpstreamRequest {
    request(
        "POST",
        "https://user:secret@mcp.example.com:8443/mcp?api_key=qsecret",
        &[
            ("host", "mcp.example.com"),
            ("authorization", "Bearer sk-abcdefghijklmnop"),
            ("content-type", "application/json"),
            ("content-length", "16"),
        ],
        BodyState::Read(b"{\"api_key\":\"x\"}".to_vec()),
    )
}

#[rstest]
#[case::newline("a\nb\tc", "a\\nb\\tc")]
#[case::controls("\u{0}\u{1}\u{1f}\u{7f}", "\\u0000\\u0001\\u001f\\u007f")]
#[case::plain("plain", "plain")]
#[case::space("a b", "a b")]
fn escaping_control_characters(#[case] text: &str, #[case] expected: &str) {
    assert_eq!(safe_text(text), expected);
}

#[test]
fn the_preview_cuts_at_five_hundred_and_twelve_characters() {
    let kept = safe_text(&"y".repeat(512));
    assert_eq!(kept.chars().count(), 512);
    let cut = safe_text(&"y".repeat(513));
    assert_eq!(cut, format!("{}...(truncated)", "y".repeat(512)));
    let escaped_cut = safe_text(&format!("{}\u{1}", "a".repeat(511)));
    assert_eq!(escaped_cut.chars().count(), 512 + 14);
}

#[rstest]
#[case::userinfo_path_and_query(
    "https://user:pass@mcp.example.com:8443/path?token=abc#frag",
    "https://mcp.example.com:8443/"
)]
#[case::bare_host("http://example.com", "http://example.com/")]
#[case::default_port("http://example.com:80/a", "http://example.com/")]
#[case::sub_path("https://mcp.example.com/sse", "https://mcp.example.com/")]
fn upstream_urls_drop_credentials_and_paths(#[case] address: &str, #[case] expected: &str) {
    assert_eq!(safe_upstream_url(&url(address)), expected);
}

#[rstest]
#[case::api_key("api_key", true)]
#[case::substring_key("monkey", true)]
#[case::cost("cost", false)]
#[case::code("code", true)]
#[case::code_verifier("code_verifier", false)]
#[case::cost_plus_token("input_cost_per_token", true)]
#[case::client_assertion("client_assertion", true)]
#[case::upper_case("API-KEY", true)]
#[case::folded_sharp_s("pa\u{df}word", true)]
fn sensitive_field_names(#[case] key: &str, #[case] expected: bool) {
    assert_eq!(sensitive_field(key), expected);
}

#[rstest]
#[case::empty(b"", "application/json", "(empty)")]
#[case::unstructured(b"plain text", "text/plain", UNSTRUCTURED_BODY)]
#[case::plain_equals(b"a=1", "text/plain", UNSTRUCTURED_BODY)]
#[case::scalar(b"123", "application/json", UNSTRUCTURED_BODY)]
#[case::bare_string(b"\"text\"", "application/json", UNSTRUCTURED_BODY)]
fn previews_without_structure(
    #[case] raw: &[u8],
    #[case] content_type: &str,
    #[case] expected: &str,
) {
    assert_eq!(preview(raw, content_type, &[], &redactor()), expected);
}

#[test]
fn a_body_past_the_capture_limit_is_omitted() {
    assert_eq!(
        preview(&vec![b'x'; 0x4001], "application/json", &[], &redactor()),
        "(omitted: body exceeds capture limit)"
    );
    assert_eq!(
        preview(&vec![b'x'; 0x4000], "application/json", &[], &redactor()),
        UNSTRUCTURED_BODY
    );
}

#[test]
fn a_sensitive_json_field_is_redacted_before_the_body_is_shown() {
    assert_eq!(
        preview(
            b"{\"api_key\":\"body-secret\"}",
            "application/json",
            &[],
            &redactor()
        ),
        "{\"REDACTED\"}"
    );
    assert_eq!(
        preview(
            b"{\"nested\":{\"token\":\"nested-secret\"}}",
            "application/json",
            &[],
            &redactor()
        ),
        "{\"nested\":{\"token\":\"REDACTED\"}}"
    );
}

#[test]
fn known_secret_values_and_secret_patterns_are_masked() {
    assert_eq!(
        preview(
            b"{\"message\":\"known-secret is here\"}",
            "application/json",
            &[SecretString::from(String::from("known-secret"))],
            &redactor()
        ),
        "{\"message\":\"REDACTED is here\"}"
    );
    assert_eq!(
        preview(
            b"{\"note\":\"Bearer sk-abcdefghijklmnopqrstuv\"}",
            "application/json",
            &[],
            &redactor()
        ),
        "{\"note\":\"REDACTED\"}"
    );
}

#[test]
fn form_bodies_keep_their_shape_and_redact_sensitive_fields() {
    assert_eq!(
        preview(
            b"a=1&api_key=form-secret&plain=visible",
            "application/x-www-form-urlencoded",
            &[],
            &redactor()
        ),
        "a=1&api_key=REDACTED&plain=visible"
    );
    assert_eq!(
        preview(
            b"token=query-secret&x=1",
            "application/x-www-form-urlencoded; charset=utf-8",
            &[SecretString::from(String::from("query-secret"))],
            &redactor()
        ),
        "token=REDACTED&x=1"
    );
    assert_eq!(
        preview(
            b"not json a=b",
            "application/x-www-form-urlencoded",
            &[],
            &redactor()
        ),
        "not+json+a=b"
    );
    assert_eq!(
        preview(
            b"a=1&flag",
            "APPLICATION/X-WWW-FORM-URLENCODED",
            &[],
            &redactor()
        ),
        "a=1&flag="
    );
}

#[test]
fn form_fields_re_encode_with_python_quote_plus() {
    assert_eq!(
        preview(
            b"x=a~b*c",
            "application/x-www-form-urlencoded",
            &[],
            &redactor()
        ),
        "x=a~b%2Ac"
    );
    assert_eq!(
        preview(
            b"api_key=a~b*c&plain=x*y",
            "application/x-www-form-urlencoded",
            &[],
            &redactor()
        ),
        "api_key=REDACTED&plain=x%2Ay"
    );
}

#[test]
fn a_tilde_secret_in_a_form_field_is_masked() {
    let known = vec![SecretString::from(String::from("tok~secret12"))];
    assert_eq!(
        preview(
            b"state=tok~secret12",
            "application/x-www-form-urlencoded",
            &known,
            &redactor()
        ),
        "state=REDACTED"
    );
    assert_eq!(
        preview(
            b"state=tok~secret12&other=1",
            "application/x-www-form-urlencoded",
            &known,
            &redactor()
        ),
        "state=REDACTED&other=1"
    );
}

#[rstest]
#[case::valid("session=abc; flag=", vec!["abc", ""])]
#[case::invalid_key("bad", vec![])]
#[case::missing_value_after_a_pair("a=1; b", vec![])]
#[case::octal_escape(r#"token="a\101b""#, vec!["aAb"])]
#[case::escaped_quote(r#"token="a\"b""#, vec!["a\"b"])]
#[case::attribute("a=1; Path=/", vec!["1"])]
#[case::repeated_key("a=1; a=2", vec!["2"])]
#[case::dollar_attribute("$Version=1; a=b", vec!["b"])]
#[case::quoted_value(r#"other="quoted value""#, vec!["quoted value"])]
#[case::comma_inside_a_value("a=1, b=2", vec!["1,", "2"])]
#[case::three_octal_digits(r#"token="a\077b""#, vec!["a?b"])]
#[case::leading_digit_out_of_range(r#"token="a\477b""#, vec!["a477b"])]
#[case::non_octal_digit(r#"token="a\108b""#, vec!["a108b"])]
#[case::two_digits_are_not_an_escape(r#"token="a\12""#, vec!["a12"])]
#[case::attribute_before_a_morsel("Path=/", vec![])]
#[case::attribute_before_a_morsel_hides_a_later_pair("Path=/; a=1", vec![])]
#[case::valueless_reserved_attribute_after_a_morsel("a=1; Path", vec![])]
#[case::flag_attribute("a=1; Secure", vec!["1"])]
fn cookie_headers_yield_their_morsels(#[case] value: &str, #[case] expected: Vec<&str>) {
    let mut secrets = header_secret_values("cookie", value);
    assert_eq!(
        secrets.first().map(SecretString::expose_secret),
        Some(value)
    );
    secrets.remove(0);
    assert_eq!(exposed(&secrets), expected);
}

#[test]
fn request_secrets_come_from_the_body_url_and_headers() {
    let secrets = request_secret_values(&request(
        "POST",
        "https://user:secret@mcp.example.com:8443/mcp?api_key=qsecret&x=1&blank=",
        &[
            ("host", "mcp.example.com"),
            ("authorization", "Bearer sk-abcdefghijklmnop"),
            ("content-type", "application/json"),
            ("cookie", "session=cookie-secret; other=plain"),
            ("content-length", "21"),
        ],
        BodyState::Read(b"{\"api_key\":\"body-secret\"}".to_vec()),
    ))
    .expect("known body");
    assert_eq!(
        exposed(&secrets),
        vec![
            "session=cookie-secret; other=plain",
            "Bearer sk-abcdefghijklmnop",
            "sk-abcdefghijklmnop",
            "cookie-secret",
            "body-secret",
            "qsecret",
            "secret",
            "plain",
            "1",
        ]
    );
}

#[rstest]
#[case::raw("known-secret", "known-secret", "REDACTED")]
#[case::plus("a+b", "a b", "REDACTED")]
#[case::percent("a%20b", "a b", "REDACTED")]
#[case::slash("x%2Fy", "x/y", "REDACTED")]
#[case::underscore_is_safe("a%5Fb", "a_b", "a%5Fb")]
#[case::dot_is_safe("a%2Eb", "a.b", "a%2Eb")]
#[case::dash_is_safe("a%2Db", "a-b", "a%2Db")]
#[case::tilde_is_safe("a%7Eb", "a~b", "a%7Eb")]
#[case::json_escaped(r#"say \"hi\""#, r#"say "hi""#, "REDACTED")]
fn every_quoted_form_of_a_known_secret_is_masked(
    #[case] text: &str,
    #[case] secret: &str,
    #[case] expected: &str,
) {
    let known = vec![SecretString::from(String::from(secret))];
    assert_eq!(mask_known_values(text, &known), expected);
    assert_eq!(
        mask_known_values("nothing to mask", &known),
        "nothing to mask"
    );
}

#[test]
fn a_root_json_string_is_a_secret_leaf() {
    let secrets = request_secret_values(&request(
        "POST",
        "https://mcp.example.com/mcp",
        &[
            ("host", "mcp.example.com"),
            ("content-type", "application/json"),
        ],
        BodyState::Read(b"\"scalar-secret\"".to_vec()),
    ))
    .expect("known body");
    assert_eq!(exposed(&secrets), vec!["scalar-secret"]);
}

#[test]
fn a_form_body_yields_its_sensitive_values() {
    let secrets = request_secret_values(&request(
        "POST",
        "https://mcp.example.com/mcp",
        &[
            ("host", "mcp.example.com"),
            ("content-type", "application/x-www-form-urlencoded"),
            ("content-length", "33"),
        ],
        BodyState::Read(b"code=form-secret&plain=visible".to_vec()),
    ))
    .expect("known body");
    assert_eq!(exposed(&secrets), vec!["form-secret"]);
}

#[test]
fn a_body_of_exactly_the_capture_limit_is_still_scanned() {
    let mut body = String::from("code=");
    body.push_str(&"x".repeat(0x4000 - 5));
    assert_eq!(body.len(), 0x4000);
    let secrets = request_secret_values(&request(
        "POST",
        "https://mcp.example.com/mcp",
        &[
            ("host", "mcp.example.com"),
            ("content-type", "application/x-www-form-urlencoded"),
        ],
        BodyState::Read(body.into_bytes()),
    ))
    .expect("exactly the limit is readable");
    assert_eq!(exposed(&secrets).len(), 1);
    assert!(
        exposed(&secrets)
            .first()
            .is_some_and(|text| text.starts_with('x'))
    );
}

#[test]
fn a_basic_password_is_unescaped_as_a_form_component() {
    let secrets = request_secret_values(&request(
        "POST",
        "https://mcp.example.com/mcp",
        &[
            ("host", "mcp.example.com"),
            ("authorization", "Basic dXNlcjpwYStzcw=="),
            ("content-length", "0"),
        ],
        BodyState::Read(Vec::new()),
    ))
    .expect("empty body is readable");
    let exposed = exposed(&secrets);
    assert!(exposed.contains(&"pa+ss"));
    assert!(exposed.contains(&"pa ss"));
}

#[test]
fn basic_credentials_contribute_every_decoded_form() {
    let secrets = request_secret_values(&request(
        "POST",
        "https://mcp.example.com/mcp",
        &[
            ("host", "mcp.example.com"),
            ("authorization", "Basic dXNlcjpwYXNz"),
            ("content-length", "0"),
        ],
        BodyState::Read(Vec::new()),
    ))
    .expect("empty body is readable");
    assert_eq!(
        exposed(&secrets),
        vec!["Basic dXNlcjpwYXNz", "dXNlcjpwYXNz", "user:pass", "pass"]
    );
}

#[rstest]
#[case::streamed(BodyState::NotRead)]
#[case::oversized(BodyState::Read(vec![b'x'; 0x4001]))]
fn an_unavailable_body_hides_the_request_secrets(#[case] body: BodyState) {
    let secrets = request_secret_values(&request(
        "POST",
        "https://mcp.example.com/mcp?token=querysecret",
        &[("host", "mcp.example.com")],
        body,
    ));
    assert!(secrets.is_none());
}

#[test]
fn a_streamed_request_body_is_named_as_such() {
    let source = request(
        "POST",
        "https://mcp.example.com/mcp",
        &[("host", "mcp.example.com")],
        BodyState::NotRead,
    );
    assert_eq!(
        request_body_preview(&source, None, &redactor()),
        "(streamed, not captured)"
    );
}

#[test]
fn an_unread_response_body_is_named_as_such() {
    let source = request(
        "GET",
        "https://mcp.example.com/mcp",
        &[("host", "mcp.example.com")],
        BodyState::Read(Vec::new()),
    );
    let secrets = request_secret_values(&source);
    let reply = response(500, &[], BodyState::NotRead, Some(source));
    assert_eq!(
        response_body_preview(&reply, secrets.as_deref(), &redactor()),
        "(not read)"
    );
}

#[test]
fn no_secrets_means_no_response_body_preview() {
    let reply = response(500, &[], BodyState::Read(b"body".to_vec()), None);
    assert_eq!(
        response_body_preview(&reply, None, &redactor()),
        CREDENTIALS_UNAVAILABLE
    );
}

#[test]
fn a_captured_preview_wins_over_the_response_body() {
    let source = request(
        "GET",
        "https://mcp.example.com/mcp",
        &[("host", "mcp.example.com")],
        BodyState::Read(Vec::new()),
    );
    let secrets = request_secret_values(&source);
    let mut reply = response(500, &[], BodyState::NotRead, Some(source));
    reply.captured = Some(String::from("captured preview"));
    assert_eq!(
        response_body_preview(&reply, secrets.as_deref(), &redactor()),
        "captured preview"
    );
}

#[test]
fn the_description_names_every_part_of_the_failed_call() {
    let reply = response(
        400,
        &[("content-type", "application/json")],
        BodyState::Read(b"{\"error\":\"bad\"}".to_vec()),
        Some(json_request()),
    );
    assert_eq!(
        describe_upstream_response(&reply, &redactor()),
        "POST https://mcp.example.com:8443/ -> HTTP 400 | request headers: content-type=application/json, content-length=16 | request body: {\"REDACTED\"} | response body: {\"error\":\"bad\"}"
    );
}

#[test]
fn debug_output_names_the_snapshot_without_its_values() {
    let state = format!("{:?}", BodyState::Read(vec![1, 2, 3]));
    assert!(state.contains("Read"));
    assert!(state.contains('3'));
    assert_eq!(format!("{:?}", BodyState::NotRead), "NotRead");
    let source = request(
        "POST",
        "https://user:secret@mcp.example.com/mcp",
        &[("authorization", "Bearer sk-secret-value")],
        BodyState::Read(vec![1, 2, 3]),
    );
    let described = format!("{source:?}");
    assert!(described.contains("POST"));
    assert!(described.contains("authorization"));
    assert!(!described.contains("sk-secret-value"));
    assert!(!described.contains("secret@"));
    let reply = response(
        400,
        &[("x-mcp-debug", "present")],
        BodyState::NotRead,
        Some(source),
    );
    let reply_text = format!("{reply:?}");
    assert!(reply_text.contains("400"));
    assert!(reply_text.contains("x-mcp-debug"));
    assert!(reply_text.contains("NotRead"));
}

#[test]
fn a_response_without_its_request_says_so() {
    let reply = response(502, &[], BodyState::Read(b"bad gateway".to_vec()), None);
    assert_eq!(
        describe_upstream_response(&reply, &redactor()),
        "HTTP 502 | request unavailable"
    );
}

#[test]
fn only_content_headers_are_shown_unmasked() {
    let map = headers(&[
        ("authorization", "Bearer sk-abcdefghijklmnop"),
        ("content-type", "application/json"),
        ("accept", "application/json"),
        ("x-custom", "custom-secret"),
    ]);
    assert_eq!(
        masked_headers(&map),
        "content-type=application/json, accept=application/json"
    );
}

#[tokio::test]
async fn a_captured_error_body_is_redacted() {
    let redactor = redactor();
    let mut reply = response(
        500,
        &[("content-type", "application/json")],
        BodyState::NotRead,
        Some(request(
            "GET",
            "https://mcp.example.com/mcp?token=querysecret",
            &[("host", "mcp.example.com")],
            BodyState::Read(Vec::new()),
        )),
    );
    let chunks = vec![Ok::<Vec<u8>, io::Error>(
        b"{\"token\":\"querysecret\"}".to_vec(),
    )];
    capture_error_response(&mut reply, stream::iter(chunks), &redactor).await;
    assert_eq!(
        reply.body,
        BodyState::Read(b"{\"token\":\"querysecret\"}".to_vec())
    );
    assert_eq!(reply.captured.as_deref(), Some("{\"token\":\"REDACTED\"}"));
}

#[tokio::test]
async fn the_capture_stops_at_the_first_failing_read() {
    let redactor = redactor();
    let mut reply = response(500, &[], BodyState::NotRead, None);
    let chunks = vec![Err::<Vec<u8>, io::Error>(io::Error::other("stream broke"))];
    capture_error_response(&mut reply, stream::iter(chunks), &redactor).await;
    assert_eq!(reply.body, BodyState::Read(Vec::new()));
    assert_eq!(reply.captured.as_deref(), Some(CAPTURE_FAILED));
}

#[tokio::test]
async fn the_capture_keeps_a_body_past_the_limit_for_the_oversize_message() {
    let redactor = redactor();
    let mut reply = response(
        500,
        &[("content-type", "text/plain")],
        BodyState::NotRead,
        Some(request(
            "GET",
            "https://mcp.example.com/mcp",
            &[("host", "mcp.example.com")],
            BodyState::Read(Vec::new()),
        )),
    );
    let chunks = vec![Ok::<Vec<u8>, io::Error>(vec![b'z'; 20000])];
    capture_error_response(&mut reply, stream::iter(chunks), &redactor).await;
    assert_eq!(reply.body, BodyState::Read(vec![b'z'; 0x4001]));
    assert_eq!(
        reply.captured.as_deref(),
        Some("(omitted: body exceeds capture limit)")
    );
}

#[tokio::test]
async fn a_successful_response_is_left_alone() {
    let redactor = redactor();
    let mut reply = response(200, &[], BodyState::NotRead, None);
    let chunks = vec![Ok::<Vec<u8>, io::Error>(b"fine".to_vec())];
    capture_error_response(&mut reply, stream::iter(chunks), &redactor).await;
    assert_eq!(reply.body, BodyState::NotRead);
    assert!(reply.captured.is_none());
}

#[tokio::test]
async fn a_capture_without_a_request_keeps_the_preview_unavailable() {
    let redactor = redactor();
    let mut reply = response(500, &[], BodyState::NotRead, None);
    let chunks = vec![Ok::<Vec<u8>, io::Error>(b"body".to_vec())];
    capture_error_response(&mut reply, stream::iter(chunks), &redactor).await;
    assert_eq!(reply.body, BodyState::Read(b"body".to_vec()));
    assert_eq!(reply.captured.as_deref(), Some(CREDENTIALS_UNAVAILABLE));
}

struct ProbeStream {
    chunks: Vec<Vec<u8>>,
    position: usize,
    polled_past_end: Arc<AtomicBool>,
}

impl Stream for ProbeStream {
    type Item = Result<Vec<u8>, io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.as_mut().get_mut();
        if let Some(chunk) = this.chunks.get(this.position) {
            this.position = this.position.saturating_add(1);
            return Poll::Ready(Some(Ok(chunk.clone())));
        }
        this.polled_past_end.store(true, Ordering::SeqCst);
        Poll::Ready(None)
    }
}

#[tokio::test(start_paused = true)]
async fn the_capture_stops_polling_once_the_limit_is_reached() {
    let redactor = redactor();
    let mut reply = response(
        500,
        &[("content-type", "text/plain")],
        BodyState::NotRead,
        None,
    );
    let polled = Arc::new(AtomicBool::new(false));
    let stream = ProbeStream {
        chunks: vec![vec![b'z'; 0x4000], vec![b'y'; 1]],
        position: 0,
        polled_past_end: Arc::clone(&polled),
    };
    capture_error_response(&mut reply, stream, &redactor).await;
    let mut expected = vec![b'z'; 0x4000];
    expected.push(b'y');
    assert_eq!(reply.body, BodyState::Read(expected));
    assert!(!polled.load(Ordering::SeqCst));
}

#[tokio::test(start_paused = true)]
async fn the_capture_gives_up_after_one_second() {
    let redactor = redactor();
    let mut reply = response(500, &[], BodyState::NotRead, None);
    let started = Instant::now();
    capture_error_response(
        &mut reply,
        stream::pending::<Result<Vec<u8>, io::Error>>(),
        &redactor,
    )
    .await;
    assert_eq!(started.elapsed(), Duration::from_secs(1));
    assert_eq!(reply.body, BodyState::Read(Vec::new()));
    assert_eq!(reply.captured.as_deref(), Some(CAPTURE_FAILED));
}

struct Node {
    response: Option<UpstreamResponse>,
    cause: Option<Box<Self>>,
}

impl UpstreamFailure for Node {
    fn response(&self) -> Option<&UpstreamResponse> {
        self.response.as_ref()
    }

    fn cause(&self) -> Option<&dyn UpstreamFailure> {
        let cause = self.cause.as_deref()?;
        Some(cause)
    }
}

fn node(status: u16) -> Node {
    Node {
        response: Some(response(status, &[], BodyState::NotRead, None)),
        cause: None,
    }
}

#[test]
fn a_failure_chain_is_described_outer_to_inner() {
    let root = Node {
        response: Some(response(429, &[], BodyState::NotRead, None)),
        cause: Some(Box::new(node(503))),
    };
    assert_eq!(
        describe_upstream_http_failure(&root, &redactor()),
        Some(String::from(
            "HTTP 429 | request unavailable | HTTP 503 | request unavailable"
        ))
    );
}

#[test]
fn a_failure_without_any_response_describes_nothing() {
    let root = Node {
        response: None,
        cause: Some(Box::new(node(503))),
    };
    assert_eq!(
        describe_upstream_http_failure(&root, &redactor()),
        Some(String::from("HTTP 503 | request unavailable"))
    );
    let empty = Node {
        response: None,
        cause: None,
    };
    assert_eq!(describe_upstream_http_failure(&empty, &redactor()), None);
}

fn chain(depth: usize) -> Node {
    if depth == 0 {
        return node(500);
    }
    Node {
        response: Some(response(500, &[], BodyState::NotRead, None)),
        cause: Some(Box::new(chain(depth.saturating_sub(1)))),
    }
}

#[test]
fn the_failure_walk_stops_after_sixteen_nodes() {
    let described = describe_upstream_http_failure(&chain(19), &redactor()).expect("descriptions");
    assert_eq!(described.matches("HTTP 500").count(), 16);
}
