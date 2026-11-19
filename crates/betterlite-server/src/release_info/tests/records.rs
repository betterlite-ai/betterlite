use rstest::rstest;
use serde::Serialize;

use crate::release_info::records::{
    LatestRelease, decode_fuzzed_latest_release, decode_latest_release,
};

#[derive(Serialize)]
struct ReleaseWire<'input> {
    tag_name: &'input str,
    html_url: &'input str,
    body: &'input str,
}

fn record(tag_name: &str, body: &str) -> String {
    let wire = ReleaseWire {
        tag_name,
        html_url: "https://example.test/r",
        body,
    };
    serde_json::to_string(&wire).expect("the release record should serialize")
}

#[rstest]
#[case::one_leading_v("v1.2.3", "1.2.3")]
#[case::only_one_v_is_stripped("vv1.2.3", "v1.2.3")]
#[case::no_v("1.2.3", "1.2.3")]
#[case::uppercase_v("V1.2.3", "V1.2.3")]
#[case::a_bare_v("v", "")]
fn strips_one_leading_lowercase_v(#[case] tag_name: &str, #[case] version: &str) {
    let release = decode_latest_release(200, record(tag_name, "").as_bytes());
    let LatestRelease::Available(info) = release else {
        panic!("a valid release record should be available");
    };
    assert_eq!(info.version, version);
    assert_eq!(info.release_url, "https://example.test/r");
}

#[test]
fn counts_the_bullets_of_the_release_body() {
    let body = "* feat: add a button\n* fix: stop a crash\n* chore: bump deps\n* plain\n* @ann made their first contribution\n";
    let release = decode_latest_release(200, record("v2.4.6", body).as_bytes());
    let LatestRelease::Available(info) = release else {
        panic!("a valid release record should be available");
    };
    assert_eq!(info.new_features, 1);
    assert_eq!(info.bug_fixes, 1);
    assert_eq!(info.other_updates, 2);
}

#[test]
fn missing_buckets_serialize_as_zero() {
    let release = decode_latest_release(200, record("v1", "* feat: only").as_bytes());
    let LatestRelease::Available(info) = release else {
        panic!("a valid release record should be available");
    };
    let text = serde_json::to_string(&info).expect("the release info should serialize");
    assert_eq!(
        text,
        r#"{"version":"1","new_features":1,"bug_fixes":0,"other_updates":0,"release_url":"https://example.test/r"}"#
    );
}

#[test]
fn a_non_200_status_is_unavailable() {
    let release = decode_latest_release(404, record("v1", "").as_bytes());
    let LatestRelease::Unavailable(unavailable) = release else {
        panic!("a non-200 status should be unavailable");
    };
    assert_eq!(unavailable.reason, "GitHub responded with status 404");
}

#[rstest]
#[case::not_json("not a release record")]
#[case::not_an_object(r#"["v1"]"#)]
#[case::null("null")]
#[case::missing_tag_name(r#"{"html_url":"https://example.test/r","body":""}"#)]
#[case::a_numeric_tag_name(r#"{"tag_name":7,"html_url":"https://example.test/r","body":""}"#)]
#[case::a_null_html_url(r#"{"tag_name":"v1","html_url":null,"body":""}"#)]
#[case::a_missing_body(r#"{"tag_name":"v1","html_url":"https://example.test/r"}"#)]
fn an_invalid_payload_is_unavailable(#[case] body: &str) {
    let release = decode_latest_release(200, body.as_bytes());
    let LatestRelease::Unavailable(unavailable) = release else {
        panic!("an invalid payload should be unavailable");
    };
    assert!(
        unavailable
            .reason
            .starts_with("GitHub release payload was not the expected shape: ")
    );
}

#[test]
fn extra_fields_are_ignored() {
    let body =
        r#"{"tag_name":"v1","html_url":"https://example.test/r","body":"","published_at":"2024"}"#;
    let release = decode_latest_release(200, body.as_bytes());
    assert!(matches!(release, LatestRelease::Available(_)));
}

#[test]
fn a_repeated_key_is_unavailable() {
    let body = r#"{"tag_name":"v1","tag_name":"v2","html_url":"https://example.test/r","body":""}"#;
    let release = decode_latest_release(200, body.as_bytes());
    assert!(matches!(release, LatestRelease::Unavailable(_)));
}

#[test]
fn a_fuzzed_status_byte_becomes_the_response_status() {
    let mut bytes = vec![0_u8];
    bytes.extend_from_slice(record("v1", "").as_bytes());
    let release = decode_fuzzed_latest_release(&bytes);
    let LatestRelease::Unavailable(unavailable) = release else {
        panic!("a non-200 status should be unavailable");
    };
    assert_eq!(unavailable.reason, "GitHub responded with status 0");
}

#[test]
fn an_empty_fuzz_input_is_an_empty_200_response() {
    let release = decode_fuzzed_latest_release(&[]);
    let LatestRelease::Unavailable(unavailable) = release else {
        panic!("an empty payload should be unavailable");
    };
    assert!(
        unavailable
            .reason
            .starts_with("GitHub release payload was not the expected shape: ")
    );
}

#[test]
fn a_fuzzed_200_status_reads_the_rest_as_the_body() {
    let mut bytes = vec![200_u8];
    bytes.extend_from_slice(record("v1", "* feat: add").as_bytes());
    let release = decode_fuzzed_latest_release(&bytes);
    let LatestRelease::Available(info) = release else {
        panic!("a 200 with a valid record should be available");
    };
    assert_eq!(info.new_features, 1);
}
