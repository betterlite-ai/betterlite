#![cfg(test)]
use crate::case::Compare;
use crate::case::decode_http_case;
use crate::compare::compare;
use crate::http::Response;
use crate::runner::DatabaseEnvironment;
use secrecy::ExposeSecret as _;
use std::collections::BTreeMap;
use std::{fs, path::Path};

#[test]
fn legacy_case_without_database() {
    decode_http_case(b"name: legacy\nconfig: config\nenv: {}\nrequest: {method: GET, path: /health/liveliness, headers: {}, body: null}\nupstream: []\ncompare: {headers: [], headers_present: [], upstream_headers: [], ignore: []}\n").unwrap();
}

#[test]
fn public_route_without_key() {
    let case = decode_http_case(
        b"name: public\nconfig: config\nenv: {}\nrequest: {method: GET, path: /health/liveliness, headers: {}, body: null}\nupstream: []\ncompare: {headers: [], headers_present: [], upstream_headers: [], ignore: []}\n",
    )
    .unwrap();
    assert_eq!(case.request.path, "/health/liveliness");
    assert!(case.request.headers.is_empty());
    assert!(case.database.is_none());
}

#[test]
fn fixture_unknown_field() {
    decode_http_case(include_bytes!(
        "../../../fuzz/corpus/http_case/postgres-unknown-field.yaml"
    ))
    .unwrap_err();
}

#[test]
fn duplicate_identity() {
    decode_http_case(include_bytes!(
        "../../../fuzz/corpus/http_case/postgres-duplicate-identity.yaml"
    ))
    .unwrap_err();
}

#[test]
fn missing_principal() {
    decode_http_case(include_bytes!(
        "../../../fuzz/corpus/http_case/postgres-missing-principal.yaml"
    ))
    .unwrap_err();
}

#[test]
fn malformed_yaml() {
    decode_http_case(include_bytes!(
        "../../../fuzz/corpus/http_case/postgres-malformed.yaml"
    ))
    .unwrap_err();
}

#[test]
fn truncated_rows() {
    decode_http_case(include_bytes!(
        "../../../fuzz/corpus/http_case/postgres-truncated-rows.yaml"
    ))
    .unwrap_err();
}

#[test]
fn empty_configured_url_is_kept_for_the_fixture() {
    let environment = <DatabaseEnvironment as clap::Parser>::try_parse_from([
        "oracle",
        "--postgres-test-url",
        "",
    ])
    .unwrap();
    let url = environment.postgres_test_url.unwrap();
    assert_eq!(url.expose_secret(), "");
}

#[test]
fn replays_the_http_case_corpus() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/http_case");
    let mut paths = fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    paths.sort();
    paths
        .into_iter()
        .try_for_each(|path| {
            let bytes = fs::read(path.as_path()).unwrap();
            let result = decode_http_case(&bytes);
            let name = path.file_name().and_then(|value| value.to_str()).unwrap();
            if name == "postgres-valid.yaml" {
                result.unwrap();
            } else {
                assert!(result.is_err(), "{} should be rejected", path.display());
            }
            Ok::<(), ()>(())
        })
        .unwrap();
}

#[test]
fn detects_status_mismatch() {
    let left = response(200, body("{\"ok\":true}"));
    let right = response(500, body("{\"ok\":true}"));
    assert_eq!(
        compare(&left, &right, &[], &[], &Compare::default())
            .unwrap_err()
            .pointer,
        "/status"
    );
}

#[test]
fn detects_error_param_mismatch() {
    let left = response(400, body("{\"error\":{\"param\":\"model\"}}"));
    let right = response(400, body("{\"error\":{\"param\":\"messages\"}}"));
    assert_eq!(
        compare(&left, &right, &[], &[], &Compare::default())
            .unwrap_err()
            .pointer,
        "/error/param"
    );
}

#[test]
fn detects_role_mismatch() {
    let left = response(200, body("{\"principal\":{\"role\":\"proxy_admin\"}}"));
    let right = response(200, body("{\"principal\":{\"role\":\"internal_user\"}}"));
    assert_eq!(
        compare(&left, &right, &[], &[], &Compare::default())
            .unwrap_err()
            .pointer,
        "/principal/role"
    );
}

fn response(status: u16, body: serde_json::Value) -> Response {
    Response {
        status,
        headers: BTreeMap::new(),
        body,
    }
}

fn body(source: &str) -> serde_json::Value {
    serde_json::from_str(source).unwrap()
}
