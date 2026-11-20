use super::support::capture_logs;
use super::support::fixture;
use super::support::request;
use super::support::saw_log;
use super::support::value;
use crate::call_wrapper::steps;

#[test]
fn debug_request_prints_the_request_line() {
    let (events, _guard) = capture_logs();
    let request = request(r#"{"model":"gpt-4o"}"#);
    steps::debug_request("completion", &request, true);
    assert!(saw_log(&events, "Request to litellm"));
}

#[test]
fn debug_request_stays_quiet_without_the_debug_flag() {
    let (events, _guard) = capture_logs();
    let request = request(r#"{"model":"gpt-4o"}"#);
    steps::debug_request("completion", &request, false);
    assert!(!saw_log(&events, "Request to litellm"));
}

#[test]
fn load_credentials_surfaces_a_bad_credential_name() {
    let fixture = fixture();
    let mut request = request(r#"{"litellm_credential_name":5}"#);
    let error = steps::load_credentials(&fixture.wrapper, &mut request)
        .expect_err("a number is not a credential name");
    assert_eq!(
        error.to_string(),
        "the litellm_credential_name value must be a string"
    );
}

#[test]
fn modify_max_tokens_skips_an_unmodified_call_type() {
    let fixture = fixture();
    let mut request = request(r#"{"model":"gpt-4o","max_tokens":10}"#);
    steps::modify_max_tokens(&fixture.wrapper, "embedding", &mut request, Some("gpt-4o"))
        .expect("the step succeeds");
    assert_eq!(request.get("max_tokens"), Some(&value("10")));
    assert!(fixture.tokens.calls().is_empty());
}
