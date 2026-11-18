#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use betterlite_oracle_helper_api::helper::Helper as _;
use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::{HelperRun, ORACLE_DIR, run_case};
use rstest::rstest;
use serde_json::{Value, json};

use super::Op;

fn input(userinfo: &Value) -> Value {
    json!({ "userinfo": userinfo })
}

#[rstest]
fn matches_the_pinned_fastapi_sso_converter(
    #[base_dir = "../../.."]
    #[files("cases/helpers/google-identity/*.yaml")]
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
fn helper_seeds_select_google_identity() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .expect("helper corpus")
        .map(|entry| entry.expect("corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("google-identity"))
        })
        .map(|path| fs::read_to_string(path).expect("seed file"))
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).expect("helper seed");
        validate_helper(Path::new("<helper>"), &case).is_ok()
            && case.call.operation == "google_identity"
            && Op::check(&case.call.input).is_ok()
    }));
}

#[test]
fn a_verified_userinfo_becomes_the_identity_fields() {
    let output = Op::decode_and_run(&input(&json!({
        "sub": "10769150350006150715113082367",
        "email": "ann@example.com",
        "email_verified": true,
        "given_name": "Ann",
        "family_name": "Lee",
        "name": "Ann Lee",
        "picture": "https://img.example.com/ann.png",
        "locale": "en",
    })))
    .unwrap();
    assert_eq!(
        output,
        json!({
            "id": "10769150350006150715113082367",
            "email": "ann@example.com",
            "first_name": "Ann",
            "last_name": "Lee",
            "display_name": "Ann Lee",
            "picture": "https://img.example.com/ann.png",
            "provider": "google",
            "team_ids": [],
            "user_role": null,
            "extra_fields": null,
        })
    );
}

#[test]
fn an_unverified_userinfo_carries_the_sso_login_status() {
    let error = Op::decode_and_run(&input(&json!({
        "email": "ann@example.com",
        "email_verified": false,
    })))
    .unwrap_err();
    assert_eq!(error.status, Some(401));
    assert_eq!(
        error.message,
        "User ann@example.com is not verified with Google"
    );
}

#[test]
fn an_input_field_beside_the_userinfo_is_rejected() {
    assert!(Op::check(&json!({"userinfo": {}, "raw": true})).is_err());
}
