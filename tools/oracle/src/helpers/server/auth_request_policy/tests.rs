#![cfg(test)]

use std::fs;
use std::ops::Div as _;
use std::path::{Path, PathBuf};

use betterlite_oracle_helper_api::helper::Helper as _;
use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::{HelperRun, ORACLE_DIR, run_case};
use rstest::rstest;
use serde_json::json;

use super::Op;

#[rstest]
fn matches_the_pinned_auth_request_policy(
    #[base_dir = "../../.."]
    #[files("cases/helpers/auth-request-policy/*.yaml")]
    path: PathBuf,
) {
    let source = fs::read_to_string(path).expect("case file");
    let case: HelperCase = serde_saphyr::from_str(&source).expect("helper case");
    assert!(matches!(
        run_case(&[crate::GROUP], &case, false).expect("case runs"),
        HelperRun::Match
    ));
}

#[test]
fn helper_seeds_select_auth_request_policy() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .expect("helper corpus")
        .map(|entry| entry.expect("corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("auth-request-policy"))
        })
        .map(|path| fs::read_to_string(path).expect("seed file"))
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).expect("helper seed");
        validate_helper(Path::new("<helper>"), &case).is_ok()
            && case.call.operation == "auth_request_policy"
            && Op::check(&case.call.input).is_ok()
    }));
}

#[test]
fn byte_projection_divides_by_a_mebibyte() {
    let one_byte_over = 1_048_577.0_f64.div(1_048_576.0_f64);
    let output = Op::decode_and_run(&json!({
        "calls": [{"function": "bytes_to_mb", "values": [0_i32, 0x0008_0000_i32, 0x0010_0000_i32, 0x0010_0001_i32]}],
    }))
    .expect("the projection runs");
    assert_eq!(output, json!([[0.0_f64, 0.5_f64, 1.0_f64, one_byte_over]]));
}

#[test]
fn ip_projection_trusts_the_whole_forwarded_header() {
    let output = Op::decode_and_run(&json!({
        "calls": [{"function": "ip", "requests": [
            {
                "forwarded_for": "10.0.0.1, 203.0.113.7",
                "client_host": "10.0.0.9",
                "use_x_forwarded_for": true,
                "allowed_ips": ["10.0.0.1"],
            },
            {
                "forwarded_for": null,
                "client_host": null,
                "use_x_forwarded_for": true,
                "allowed_ips": null,
            },
        ]}],
    }))
    .expect("the projection runs");
    assert_eq!(
        output,
        json!([[
            {
                "ip": "10.0.0.1, 203.0.113.7",
                "allowed": false,
                "passed_in_ip": "10.0.0.1, 203.0.113.7",
            },
            {"ip": "", "allowed": true, "passed_in_ip": null},
        ]])
    );
}

#[test]
fn classification_requires_the_marker_and_a_401() {
    let output = Op::decode_and_run(&json!({
        "calls": [{"function": "classify", "rows": [
            {"kind": "proxy", "code": "401", "status_code": 401_i32, "marker": "exact_true", "message": "m"},
            {"kind": "proxy", "code": "401", "status_code": 401_i32, "marker": "absent", "message": "m"},
            {"kind": "http", "code": "403", "status_code": 401_i32, "marker": "exact_true", "message": "m"},
            {"kind": "other", "code": "401", "status_code": 401_i32, "marker": "exact_true", "message": "m"},
        ]}],
    }))
    .expect("the projection runs");
    assert_eq!(output, json!([[true, false, false, false]]));
}

#[test]
fn marking_returns_a_copy() {
    let output = Op::decode_and_run(&json!({
        "calls": [{"function": "mark", "rows": [
            {
                "is_invalid_virtual_key": true,
                "code": "401",
                "message": "Invalid proxy server token passed.",
                "openai_code": null,
                "headers": {"x-trace": "a"},
            },
        ]}],
    }))
    .expect("the projection runs");
    assert_eq!(
        output,
        json!([[
            {
                "same_object": false,
                "classified": true,
                "type": "auth_error",
                "param": "None",
                "code": "401",
                "openai_code": "401",
                "headers": {"x-trace": "a"},
            },
        ]])
    );
}

#[test]
fn marking_a_disabled_row_returns_the_original() {
    let output = Op::decode_and_run(&json!({
        "calls": [{"function": "mark", "rows": [
            {
                "is_invalid_virtual_key": false,
                "code": "401",
                "message": "Invalid proxy server token passed.",
                "openai_code": null,
                "headers": {},
            },
        ]}],
    }))
    .expect("the projection runs");
    assert_eq!(
        output,
        json!([[
            {
                "same_object": true,
                "classified": false,
                "type": "auth_error",
                "param": "None",
                "code": "401",
                "openai_code": "401",
                "headers": {},
            },
        ]])
    );
}

#[test]
fn pre_db_denies_an_unlisted_ip_with_auth_error_403() {
    let output = Op::decode_and_run(&json!({
        "calls": [{"function": "pre_db", "requests": [{
            "allowed_ips": ["10.0.0.1"],
            "allowed_routes": null,
            "use_x_forwarded_for": true,
            "route": "/v1/chat/completions",
            "forwarded_for": "203.0.113.9",
            "client_host": null,
            "content_length": "2",
            "max_request_size_mb": null,
            "body": {"model": "gpt-4o"},
        }]}],
    }))
    .expect("the projection runs");
    assert_eq!(
        output,
        json!([[
            {
                "ok": false,
                "status": 403_i32,
                "type": "auth_error",
                "param": "None",
                "code": "403",
                "message": "Access forbidden: IP address 203.0.113.9 not allowed.",
            },
        ]])
    );
}

#[test]
fn pre_db_accepts_without_routes() {
    let output = Op::decode_and_run(&json!({
        "calls": [{"function": "pre_db", "requests": [
            {
                "allowed_ips": null,
                "allowed_routes": null,
                "use_x_forwarded_for": false,
                "route": "/v1/chat/completions",
                "forwarded_for": null,
                "client_host": null,
                "content_length": "2",
                "max_request_size_mb": null,
                "body": {"model": "gpt-4o"},
            },
        ]}],
    }))
    .expect("the projection runs");
    assert_eq!(
        output,
        json!([[
            {
                "ok": true,
                "status": null,
                "type": null,
                "param": null,
                "code": null,
                "message": null,
            },
        ]])
    );
}

#[test]
fn pre_db_denies_an_empty_allowlist() {
    let output = Op::decode_and_run(&json!({
        "calls": [{"function": "pre_db", "requests": [
            {
                "allowed_ips": null,
                "allowed_routes": [],
                "use_x_forwarded_for": false,
                "route": "/v1/chat/completions",
                "forwarded_for": null,
                "client_host": null,
                "content_length": "2",
                "max_request_size_mb": null,
                "body": {"model": "gpt-4o"},
            },
        ]}],
    }))
    .expect("the projection runs");
    assert_eq!(
        output,
        json!([[
            {
                "ok": false,
                "status": 403_i32,
                "type": "auth_error",
                "param": "None",
                "code": "403",
                "message": "Access forbidden: Route /v1/chat/completions not allowed",
            },
        ]])
    );
}
