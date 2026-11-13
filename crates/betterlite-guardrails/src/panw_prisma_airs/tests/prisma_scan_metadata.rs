use rstest::rstest;

use crate::panw_prisma_airs::metadata::{RequestData, ScanMetadata, prepare_metadata_from_request};

fn parse(json: &str) -> ScanMetadata {
    let data: RequestData = serde_json::from_str(json).expect("request data parses");
    prepare_metadata_from_request(&data)
}

#[test]
fn an_empty_request_gets_the_defaults() {
    assert_eq!(
        parse("{}"),
        ScanMetadata {
            user: "litellm_user".to_owned(),
            model: "unknown".to_owned(),
            profile_name: None,
            profile_id: None,
            user_ip: None,
            requester_ip_address: None,
            app_name: None,
            app_user: None,
            litellm_trace_id: None,
        }
    );
}

#[test]
fn request_fields_and_user_metadata_are_copied() {
    let metadata = parse(
        r#"{"user":"u","model":"gpt-4o","metadata":{"profile_name":"p","profile_id":"pid","user_ip":"1.2.3.4","app_name":"shop","app_user":"alice"}}"#,
    );
    assert_eq!(metadata.user, "u");
    assert_eq!(metadata.model, "gpt-4o");
    assert_eq!(metadata.profile_name.as_deref(), Some("p"));
    assert_eq!(metadata.profile_id.as_deref(), Some("pid"));
    assert_eq!(metadata.user_ip.as_deref(), Some("1.2.3.4"));
    assert_eq!(metadata.app_name.as_deref(), Some("shop"));
    assert_eq!(metadata.app_user.as_deref(), Some("alice"));
}

#[test]
fn requester_metadata_fills_what_user_metadata_lacks() {
    let metadata = parse(
        r#"{"metadata":{"profile_name":"outer","requester_metadata":{"profile_name":"inner","profile_id":"pid","user_ip":"9.9.9.9","app_name":"app","app_user":"bob"}}}"#,
    );
    assert_eq!(metadata.profile_name.as_deref(), Some("outer"));
    assert_eq!(metadata.profile_id.as_deref(), Some("pid"));
    assert_eq!(metadata.user_ip.as_deref(), Some("9.9.9.9"));
    assert_eq!(metadata.app_name.as_deref(), Some("app"));
    assert_eq!(metadata.app_user.as_deref(), Some("bob"));
}

#[rstest]
#[case::request_level_id_wins(
    r#"{"litellm_trace_id":"top","metadata":{"litellm_trace_id":"meta","trace_id":"header","requester_metadata":{"litellm_trace_id":"req"}}}"#,
    "top"
)]
#[case::metadata_litellm_trace_id(
    r#"{"metadata":{"litellm_trace_id":"meta","trace_id":"header","requester_metadata":{"litellm_trace_id":"req"}}}"#,
    "meta"
)]
#[case::header_trace_id(
    r#"{"metadata":{"trace_id":"header","requester_metadata":{"litellm_trace_id":"req"}}}"#,
    "header"
)]
#[case::requester_metadata(
    r#"{"metadata":{"requester_metadata":{"litellm_trace_id":"req"}}}"#,
    "req"
)]
fn the_trace_id_is_taken_in_priority_order(#[case] json: &str, #[case] expected: &str) {
    assert_eq!(parse(json).litellm_trace_id.as_deref(), Some(expected));
}

#[test]
fn an_explicit_empty_user_is_kept() {
    assert_eq!(parse(r#"{"user":""}"#).user, "");
}

#[test]
fn a_wrong_typed_field_is_an_error_naming_it() {
    let error = serde_json::from_str::<RequestData>(r#"{"user":5}"#).expect_err("must fail");
    assert!(error.to_string().contains("invalid type"), "{error}");
}
