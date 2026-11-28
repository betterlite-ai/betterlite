#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use betterlite_oracle_helper_api::error::ErrorCode;
use betterlite_oracle_helper_api::helper::Helper as _;
use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::{HelperRun, ORACLE_DIR, run_case};
use rstest::rstest;
use serde_json::{Value, json};

use super::Op;

fn input(environment: &Value) -> Value {
    json!({
        "client_id": "microsoft-client-id",
        "redirect_url": "https://proxy.example.test/sso/callback",
        "environment": environment,
    })
}

#[rstest]
fn matches_the_pinned_custom_microsoft_sso(
    #[base_dir = "../../.."]
    #[files("cases/helpers/microsoft-sso-client/*.yaml")]
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
fn helper_seeds_select_b10148() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .expect("helper corpus")
        .map(|entry| entry.expect("corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("microsoft-sso-client"))
        })
        .map(|path| fs::read_to_string(path).expect("seed file"))
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).expect("helper seed");
        validate_helper(Path::new("<helper>"), &case).is_ok()
            && case.call.operation == "b10148"
            && Op::check(&case.call.input).is_ok()
    }));
}

#[test]
fn a_named_tenant_and_overrides_reach_the_output() {
    let output = Op::decode_and_run(&input(&json!({
        "MICROSOFT_CLIENT_SECRET": "microsoft-secret",
        "MICROSOFT_TENANT": "contoso.example.test",
        "MICROSOFT_TOKEN_ENDPOINT": "https://login.example.test/token",
    })))
    .unwrap();
    assert_eq!(
        output,
        json!({
            "tenant": "contoso.example.test",
            "discovery": {
                "authorization_endpoint": "https://login.microsoftonline.com/contoso.example.test/oauth2/v2.0/authorize",
                "token_endpoint": "https://login.example.test/token",
                "userinfo_endpoint": "https://graph.microsoft.com/v1.0/me",
            },
        })
    );
}

#[test]
fn a_missing_secret_is_reported_with_the_client_status() {
    let error = Op::decode_and_run(&input(&json!({}))).unwrap_err();
    assert_eq!(error.status, Some(500));
    assert_eq!(
        error.param,
        Some(Some("MICROSOFT_CLIENT_SECRET".to_owned()))
    );
    assert_eq!(error.code, Some(Some(ErrorCode::Text("500".to_owned()))));
}
