#![cfg(test)]

use betterlite_oracle_helper_api::helper::Helper;
use serde_json::{Value, json};

use super::{Input, Op};

const ENDPOINT: &str = "https://idp.example/token";
const TOKEN: i64 = 2_000_000_000;

fn run(input: Value) -> Value {
    let decoded: Input = serde_json::from_value(input).expect("the case input decodes");
    <Op as Helper>::run(&decoded).expect("the helper answers")
}

fn claims(exp: i64) -> Value {
    json!({"iss": "https://idp.example", "sub": "user-1", "exp": exp})
}

fn vars(include_client_id: Option<&str>) -> Value {
    let mut vars = json!({
        "GENERIC_TOKEN_ENDPOINT": ENDPOINT,
        "GENERIC_CLIENT_ID": "client-id",
        "GENERIC_CLIENT_SECRET": "client-secret",
    });
    if let Some(flag) = include_client_id {
        vars["GENERIC_INCLUDE_CLIENT_ID"] = json!(flag);
    }
    vars
}

#[test]
fn client_config_reads_the_generic_variables() {
    let answer = run(json!({"function": "client_config", "vars": vars(Some("TRUE"))}));
    assert_eq!(answer["config"]["token_endpoint"], json!(ENDPOINT));
    assert_eq!(answer["config"]["client_id"], json!("client-id"));
    assert_eq!(answer["config"]["auth_method"], json!("client_secret_post"));
    assert_eq!(answer["config"]["client_secret_len"], json!(13_u64));
    let basic = run(json!({"function": "client_config", "vars": vars(None)}));
    assert_eq!(basic["config"]["auth_method"], json!("client_secret_basic"));
}

#[test]
fn client_config_is_absent_without_a_secret() {
    let answer = run(json!({
        "function": "client_config",
        "vars": {"GENERIC_TOKEN_ENDPOINT": ENDPOINT, "GENERIC_CLIENT_ID": "client-id"},
    }));
    assert_eq!(answer, json!({"config": null}));
}

#[test]
fn refresh_posts_the_carried_token_with_basic_auth() {
    let answer = run(json!({
        "function": "refresh",
        "user_id": "user-1",
        "assertion": {"claims": claims(TOKEN), "refresh_token": "carried-token"},
        "vars": vars(None),
        "outcome": {"kind": "ok", "body": {"claims": claims(TOKEN + 3_600_i64), "refresh_token": "rotated-token"}},
    }));
    assert_eq!(answer["result"], json!("renewed"));
    assert_eq!(answer["posts"], json!(1_u64));
    assert_eq!(answer["url"], json!(ENDPOINT));
    assert_eq!(
        answer["form"],
        json!([
            ["grant_type", "refresh_token"],
            ["refresh_token", "carried-token"],
        ])
    );
    assert_eq!(
        answer["headers"],
        json!([["Authorization", "Basic Y2xpZW50LWlkOmNsaWVudC1zZWNyZXQ="]])
    );
    assert_eq!(answer["writes"][0]["user_id"], json!("user-1"));
    assert_eq!(
        answer["writes"][0]["assertion"]["refresh_token"],
        json!("rotated-token")
    );
    assert_eq!(
        answer["writes"][0]["assertion"]["expires_at_ms"],
        json!((TOKEN + 3_600_i64) * 1_000_i64)
    );
}

#[test]
fn refresh_checks_the_missing_refresh_token_before_the_missing_client() {
    let answer = run(json!({
        "function": "refresh",
        "user_id": "user-1",
        "assertion": {"claims": claims(TOKEN)},
        "vars": {},
        "outcome": {"kind": "ok"},
    }));
    assert_eq!(answer["result"], json!("rejected"));
    assert_eq!(
        answer["detail"],
        json!("no refresh token was captured at sign-in")
    );
    assert_eq!(answer["posts"], json!(0_u64));
    assert_eq!(answer["assertion"], Value::Null);
}

#[test]
fn a_newer_row_is_kept_instead_of_overwritten() {
    let answer = run(json!({
        "function": "refresh",
        "user_id": "user-1",
        "assertion": {"claims": claims(TOKEN), "refresh_token": "carried-token"},
        "row": {"claims": claims(TOKEN + 30_i64), "refresh_token": "other-token"},
        "vars": vars(None),
        "outcome": {"kind": "ok", "body": {"claims": claims(TOKEN + 3_600_i64)}},
    }));
    assert_eq!(answer["result"], json!("renewed"));
    assert_eq!(answer["writes"], json!([]));
}

#[test]
fn a_failed_write_back_is_unavailable() {
    let answer = run(json!({
        "function": "refresh",
        "user_id": "user-1",
        "assertion": {"claims": claims(TOKEN), "refresh_token": "carried-token"},
        "vars": vars(None),
        "outcome": {"kind": "ok", "body": {"claims": claims(TOKEN + 3_600_i64)}},
        "failure": "write",
    }));
    assert_eq!(answer["result"], json!("unavailable"));
    assert_eq!(
        answer["detail"],
        json!("the renewed IdP identity assertion could not be persisted")
    );
    assert_eq!(answer["writes"], json!([]));
}

#[test]
fn store_renews_an_expiring_row() {
    let answer = run(json!({
        "function": "store",
        "user_id": "user-1",
        "now_ms": TOKEN * 1_000_i64,
        "rows": {"user-1": {"claims": claims(TOKEN - 10_i64), "refresh_token": "carried-token"}},
        "vars": vars(None),
        "outcome": {"kind": "ok", "body": {"claims": claims(TOKEN + 3_600_i64)}},
    }));
    assert_eq!(answer["result"], json!("assertion"));
    assert_eq!(answer["posts"], json!(1_u64));
    assert_eq!(
        answer["assertion"]["id_token"],
        answer["writes"][0]["assertion"]["id_token"]
    );
    assert_eq!(
        answer["assertion"]["expires_at_ms"].as_i64(),
        Some((TOKEN + 3_600_i64) * 1_000_i64)
    );
}

#[test]
fn store_returns_a_fresh_row_untouched() {
    let answer = run(json!({
        "function": "store",
        "user_id": "user-1",
        "now_ms": TOKEN * 1_000_i64,
        "rows": {"user-1": {"claims": claims(TOKEN + 61_i64)}},
        "vars": vars(None),
        "outcome": {"kind": "ok"},
    }));
    assert_eq!(answer["result"], json!("assertion"));
    assert_eq!(answer["posts"], json!(0_u64));
    assert_eq!(answer["writes"], json!([]));
}

#[test]
fn store_reports_an_outage_and_a_losers_retry() {
    let outage = run(json!({
        "function": "store",
        "user_id": "user-1",
        "now_ms": TOKEN * 1_000_i64,
        "rows": {"user-1": {"claims": claims(TOKEN - 10_i64), "refresh_token": "carried-token"}},
        "vars": vars(None),
        "outcome": {"kind": "unavailable", "detail": "the IdP token endpoint is unreachable"},
    }));
    assert_eq!(outage["result"], json!("unavailable"));
    assert_eq!(
        outage["detail"],
        json!("the IdP token endpoint is unreachable")
    );
    let loser = run(json!({
        "function": "store",
        "user_id": "user-1",
        "now_ms": TOKEN * 1_000_i64,
        "rows": {"user-1": {"claims": claims(TOKEN - 10_i64)}},
        "vars": vars(None),
        "outcome": {"kind": "ok"},
        "loser": true,
    }));
    assert_eq!(loser["result"], json!("unavailable"));
    assert_eq!(
        loser["detail"],
        json!(
            "the IdP identity assertion for user_id=user-1 was being renewed by another replica and is not yet current; retry shortly"
        )
    );
    assert_eq!(loser["posts"], json!(0_u64));
}
