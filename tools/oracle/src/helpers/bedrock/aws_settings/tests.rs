#![cfg(test)]

use std::fs;
use std::path::Path;

use rstest::rstest;
use serde_json::{Value, json};

use super::Op;
use crate::case::{HelperCase, helper::validate_helper};
use crate::helpers::Helper as _;

#[test]
fn helper_seeds_select_bedrock_aws_settings() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("bedrock_aws_settings"))
        })
        .map(|path| fs::read_to_string(path).unwrap())
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        serde_saphyr::from_str(source)
            .map_err(|error| error.to_string())
            .and_then(|parsed| {
                validate_helper(Path::new("<helper>"), &parsed).map_err(|error| error.to_string())
            })
            .is_ok()
            && serde_saphyr::from_str::<HelperCase>(source)
                .is_ok_and(|case| case.call.operation == "bedrock_aws_settings")
    }));
}

#[rstest]
#[case::web_identity(
    json!({
        "aws_web_identity_token": "oidc/env/BEDROCK_TOKEN",
        "aws_role_name": "arn:aws:iam::123:role/demo",
        "aws_session_name": "session",
        "environment": {
            "BEDROCK_TOKEN": "fixture-token",
            "AWS_ACCESS_KEY_ID": "signer",
            "AWS_SECRET_ACCESS_KEY": "signer-secret",
        },
    }),
    "web_identity",
)]
#[case::role(
    json!({
        "aws_role_name": "arn:aws:iam::123:role/demo",
        "aws_session_name": "session",
        "environment": {"AWS_ACCESS_KEY_ID": "signer", "AWS_SECRET_ACCESS_KEY": "signer-secret"},
    }),
    "role",
)]
#[case::profile(
    json!({
        "aws_profile_name": "profile",
        "environment": {"AWS_SHARED_CREDENTIALS_FILE": "/credentials"},
        "files": {"/credentials": "[profile]\naws_access_key_id = profile\naws_secret_access_key = private\n"},
    }),
    "profile",
)]
#[case::session_token(
    json!({
        "aws_access_key_id": "access",
        "aws_secret_access_key": "secret",
        "aws_session_token": "token",
    }),
    "session_token",
)]
#[case::static_pair(
    json!({
        "aws_access_key_id": "access",
        "aws_secret_access_key": "secret",
        "aws_region_name": "us-west-2",
    }),
    "static",
)]
#[case::environment(
    json!({"environment": {"AWS_ACCESS_KEY_ID": "ambient", "AWS_SECRET_ACCESS_KEY": "private"}}),
    "env",
)]
#[case::role_before_profile(
    json!({
        "aws_role_name": "arn:aws:iam::123:role/demo",
        "aws_session_name": "session",
        "aws_profile_name": "profile",
        "environment": {
            "AWS_ACCESS_KEY_ID": "signer",
            "AWS_SECRET_ACCESS_KEY": "signer-secret",
            "AWS_SHARED_CREDENTIALS_FILE": "/credentials",
        },
        "files": {"/credentials": "[profile]\naws_access_key_id = profile\naws_secret_access_key = private\n"},
    }),
    "role",
)]
#[case::profile_before_session_token(
    json!({
        "aws_profile_name": "profile",
        "aws_access_key_id": "access",
        "aws_secret_access_key": "secret",
        "aws_session_token": "token",
        "environment": {"AWS_SHARED_CREDENTIALS_FILE": "/credentials"},
        "files": {"/credentials": "[profile]\naws_access_key_id = profile\naws_secret_access_key = private\n"},
    }),
    "profile",
)]
#[case::session_token_before_static(
    json!({
        "aws_access_key_id": "access",
        "aws_secret_access_key": "secret",
        "aws_session_token": "token",
        "aws_region_name": "us-west-2",
    }),
    "session_token",
)]
fn auth_selector_reports_the_source_the_resolver_ran(#[case] params: Value, #[case] source: &str) {
    let input = json!({"operation": "auth_selector", "params": params});
    let output = Op::decode_and_run(&input).unwrap();
    assert_eq!(output["source"], source);
}

#[rstest]
#[case::static_pair(
    json!({
        "settings": {"aws_access_key_id": "static", "aws_secret_access_key": "private", "aws_region_name": "us-west-2"},
        "environment": {},
        "files": {},
        "fixture": ["static", "private", null],
    }),
    "static",
    json!(3_540_u64),
)]
#[case::environment(
    json!({
        "settings": {},
        "environment": {"AWS_ACCESS_KEY_ID": "ambient", "AWS_SECRET_ACCESS_KEY": "private"},
        "files": {},
        "fixture": ["ambient", "private", null],
    }),
    "env",
    json!(600_u64),
)]
#[case::profile(
    json!({
        "settings": {"aws_profile_name": "profile"},
        "environment": {"AWS_SHARED_CREDENTIALS_FILE": "/credentials"},
        "files": {"/credentials": "[profile]\naws_access_key_id = profile\naws_secret_access_key = private\n"},
        "fixture": ["profile", "private", null],
    }),
    "profile",
    json!(null),
)]
#[case::session_token(
    json!({
        "settings": {"aws_access_key_id": "access", "aws_secret_access_key": "secret", "aws_session_token": "token"},
        "environment": {},
        "files": {},
        "fixture": ["access", "secret", "token"],
    }),
    "session_token",
    json!(null),
)]
#[case::profile_over_static(
    json!({
        "settings": {
            "aws_profile_name": "profile",
            "aws_access_key_id": "ignored",
            "aws_secret_access_key": "ignored",
            "aws_region_name": "us-west-2",
        },
        "environment": {"AWS_SHARED_CREDENTIALS_FILE": "/credentials"},
        "files": {"/credentials": "[profile]\naws_access_key_id = profile\naws_secret_access_key = private\n"},
        "fixture": ["profile", "private", null],
    }),
    "profile",
    json!(null),
)]
#[case::session_token_with_region(
    json!({
        "settings": {
            "aws_access_key_id": "access",
            "aws_secret_access_key": "secret",
            "aws_session_token": "token",
            "aws_region_name": "us-west-2",
        },
        "environment": {},
        "files": {},
        "fixture": ["access", "secret", "token"],
    }),
    "session_token",
    json!(null),
)]
fn local_reports_the_source_and_cache_ttl_the_resolver_produced(
    #[case] params: Value,
    #[case] source: &str,
    #[case] ttl: Value,
) {
    let input = json!({"operation": "local", "params": params});
    let output = Op::decode_and_run(&input).unwrap();
    assert_eq!(output["source"], source);
    assert_eq!(output["ttl"], ttl);
}
