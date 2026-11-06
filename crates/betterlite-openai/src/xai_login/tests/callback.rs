use rstest::rstest;
use secrecy::ExposeSecret;

use crate::xai_login::callback::{CallbackError, MAX_TARGET_BYTES, decode_xai_callback};

#[rstest]
#[case::first_value_wins("/callback?code=first&code=second", Some("first"))]
#[case::blank_first_keeps_the_next("/callback?code=&code=ok", Some("ok"))]
#[case::only_blank_is_absent("/callback?code=", None)]
#[case::percent_escapes("/callback?code=a%2Fb%20c", Some("a/b c"))]
#[case::plus_is_a_space("/callback?code=a+b", Some("a b"))]
#[case::missing("/callback?state=state", None)]
#[case::fragment_hides_the_query("/callback#frag?code=x", None)]
fn decodes_the_first_nonblank_code(#[case] target: &str, #[case] expected: Option<&str>) {
    let request = decode_xai_callback(target.as_bytes()).unwrap();
    assert_eq!(request.path, "/callback");
    let code = request
        .fields
        .code
        .as_ref()
        .map(ExposeSecret::expose_secret);
    assert_eq!(code, expected);
}

#[test]
fn decodes_every_callback_field() {
    let request =
        decode_xai_callback(b"/callback?code=c&state=s&error=e&error_description=d").unwrap();
    assert_eq!(
        request
            .fields
            .code
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("c")
    );
    assert_eq!(request.fields.state.as_deref(), Some("s"));
    assert_eq!(request.fields.error.as_deref(), Some("e"));
    assert_eq!(request.fields.error_description.as_deref(), Some("d"));
}

#[rstest]
#[case::other_path("/favicon.ico?code=x", "/favicon.ico")]
#[case::trailing_segment("/callback/extra", "/callback/extra")]
#[case::no_leading_slash("callback?code=x", "/x/callback")]
#[case::missing_target("", "/x/")]
#[case::root("/", "/")]
#[case::encoded_path("/call%62ack", "/call%62ack")]
#[case::absolute_form("http://127.0.0.1:56121/callback?code=x", "/callback")]
fn resolves_the_request_path(#[case] target: &str, #[case] expected: &str) {
    let request = decode_xai_callback(target.as_bytes()).unwrap();
    assert_eq!(request.path, expected);
}

#[test]
fn rejects_a_target_over_the_bound() {
    let mut target = String::from("/callback?code=");
    target.push_str(&"a".repeat(MAX_TARGET_BYTES));
    assert!(matches!(
        decode_xai_callback(target.as_bytes()),
        Err(CallbackError::TooLong)
    ));
}

#[test]
fn accepts_a_target_at_the_bound() {
    let prefix = "/callback?code=";
    let mut target = String::from(prefix);
    target.push_str(&"a".repeat(MAX_TARGET_BYTES - prefix.len()));
    let request = decode_xai_callback(target.as_bytes()).unwrap();
    let code = request.fields.code.unwrap();
    assert_eq!(code.expose_secret().len(), MAX_TARGET_BYTES - prefix.len());
}

#[test]
fn rejects_a_target_that_is_not_utf8() {
    assert!(matches!(
        decode_xai_callback(b"/callback?code=\xff"),
        Err(CallbackError::Malformed)
    ));
}
