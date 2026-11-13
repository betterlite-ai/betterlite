#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;

use super::{
    FastapiInstall, GuardrailCallback, GuardrailRaisedException, HttpException, RaisedError,
    add_guardrail_context, is_fastapi_http_exception, pre_call_rejection,
};

fn json(source: &str) -> GuardrailValue {
    serde_json::from_str(source).unwrap()
}

fn http(status_code: u16, detail: &str) -> RaisedError {
    RaisedError::Http(HttpException {
        status_code,
        detail: json(detail),
    })
}

fn callback(guardrail_name: Option<&str>, event_hook: Option<&str>) -> GuardrailCallback {
    GuardrailCallback {
        guardrail_name: guardrail_name.map(str::to_owned),
        event_hook: event_hook.map(json),
    }
}

#[rstest]
#[case::blocked_status(http(400, "{}"), &[400, 403], FastapiInstall::Installed, true)]
#[case::second_blocked_status(http(403, "{}"), &[400, 403], FastapiInstall::Installed, true)]
#[case::other_status(http(500, "{}"), &[400, 403], FastapiInstall::Installed, false)]
#[case::no_blocked_statuses(http(400, "{}"), &[], FastapiInstall::Installed, false)]
#[case::without_fastapi(http(400, "{}"), &[400], FastapiInstall::Absent, false)]
#[case::guardrail_exception_is_not_http(
    pre_call_rejection("no", Some("g"), FastapiInstall::Absent),
    &[400],
    FastapiInstall::Installed,
    false
)]
#[case::other_error(RaisedError::Other, &[400], FastapiInstall::Installed, false)]
fn only_a_blocked_http_exception_counts(
    #[case] error: RaisedError,
    #[case] codes: &[u16],
    #[case] fastapi: FastapiInstall,
    #[case] expected: bool,
) {
    assert_eq!(is_fastapi_http_exception(&error, codes, fastapi), expected);
}

#[rstest]
#[case::both_added(
    r#"{"error":"blocked"}"#,
    Some("pii"),
    Some(r#""pre_call""#),
    r#"{"error":"blocked","guardrail_name":"pii","guardrail_mode":"pre_call"}"#
)]
#[case::name_only(
    r#"{"error":"blocked"}"#,
    Some("pii"),
    None,
    r#"{"error":"blocked","guardrail_name":"pii"}"#
)]
#[case::mode_only(
    r#"{"error":"blocked"}"#,
    None,
    Some(r#""post_call""#),
    r#"{"error":"blocked","guardrail_mode":"post_call"}"#
)]
#[case::neither(r#"{"error":"blocked"}"#, None, None, r#"{"error":"blocked"}"#)]
#[case::existing_name_is_kept(
    r#"{"guardrail_name":"mine","guardrail_mode":"during_call"}"#,
    Some("pii"),
    Some(r#""pre_call""#),
    r#"{"guardrail_name":"mine","guardrail_mode":"during_call"}"#
)]
#[case::null_name_in_detail_is_kept(
    r#"{"guardrail_name":null}"#,
    Some("pii"),
    None,
    r#"{"guardrail_name":null}"#
)]
#[case::empty_name_is_not_added("{}", Some(""), None, "{}")]
#[case::empty_mode_text_is_not_added("{}", None, Some(r#""""#), "{}")]
#[case::null_mode_is_not_added("{}", None, Some("null"), "{}")]
#[case::empty_mode_list_is_not_added("{}", None, Some("[]"), "{}")]
#[case::mode_list_is_added(
    "{}",
    None,
    Some(r#"["pre_call","post_call"]"#),
    r#"{"guardrail_mode":["pre_call","post_call"]}"#
)]
#[case::mode_object_is_added(
    "{}",
    None,
    Some(r#"{"tags":{}}"#),
    r#"{"guardrail_mode":{"tags":{}}}"#
)]
#[case::mode_number_is_added("{}", None, Some("0"), r#"{"guardrail_mode":0}"#)]
fn the_detail_object_gains_the_guardrail_context(
    #[case] detail: &str,
    #[case] guardrail_name: Option<&str>,
    #[case] event_hook: Option<&str>,
    #[case] expected: &str,
) {
    let mut error = http(400, detail);
    add_guardrail_context(
        &mut error,
        &callback(guardrail_name, event_hook),
        FastapiInstall::Installed,
    );
    assert_eq!(error, http(400, expected));
}

#[rstest]
#[case::text_detail(http(400, r#""blocked""#))]
#[case::list_detail(http(400, "[1]"))]
#[case::null_detail(http(400, "null"))]
#[case::guardrail_exception(pre_call_rejection("no", None, FastapiInstall::Absent))]
#[case::other_error(RaisedError::Other)]
fn nothing_else_is_touched(#[case] error: RaisedError) {
    let mut enriched = error.clone();
    add_guardrail_context(
        &mut enriched,
        &callback(Some("pii"), Some(r#""pre_call""#)),
        FastapiInstall::Installed,
    );
    assert_eq!(enriched, error);
}

#[test]
fn without_fastapi_the_detail_is_left_alone() {
    let mut error = http(400, r#"{"error":"blocked"}"#);
    add_guardrail_context(
        &mut error,
        &callback(Some("pii"), Some(r#""pre_call""#)),
        FastapiInstall::Absent,
    );
    assert_eq!(error, http(400, r#"{"error":"blocked"}"#));
}

#[test]
fn a_rejection_is_a_400_with_the_message_as_error() {
    assert_eq!(
        pre_call_rejection("Violated policy", Some("pii"), FastapiInstall::Installed),
        http(400, r#"{"error":"Violated policy"}"#)
    );
}

#[rstest]
#[case::named(Some("pii"))]
#[case::unnamed(None)]
fn without_fastapi_a_rejection_is_a_guardrail_exception(#[case] guardrail_name: Option<&str>) {
    let expected = RaisedError::Guardrail(GuardrailRaisedException {
        guardrail_name: guardrail_name.map(str::to_owned),
        message: "Violated policy".to_owned(),
        status_code: 400,
        blocked_content: false,
    });
    assert_eq!(
        pre_call_rejection("Violated policy", guardrail_name, FastapiInstall::Absent),
        expected
    );
}

#[test]
fn the_guardrail_exception_displays_the_bare_message() {
    let RaisedError::Guardrail(exception) =
        pre_call_rejection("Violated policy", Some("pii"), FastapiInstall::Absent)
    else {
        panic!("expected a guardrail exception");
    };
    assert_eq!(exception.to_string(), "Violated policy");
}
