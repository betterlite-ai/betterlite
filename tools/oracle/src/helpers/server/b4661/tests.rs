#![cfg(test)]

use serde_json::{Value, json};

use super::{Input, Op};
use crate::helpers::Helper;

fn run(input: Value) -> Value {
    let decoded: Input = serde_json::from_value(input).expect("the case input decodes");
    <Op as Helper>::run(&decoded).expect("the helper answers")
}

fn config() -> Value {
    json!({
        "client_id": "client",
        "client_secret": "secret",
        "token_url": "https://idp.example.com/token",
        "scopes": ["read", "write"],
    })
}

#[test]
fn grant_reports_the_form_order_and_the_identity_key() {
    let mut config = config();
    config["audience"] = json!("api://aud");
    config["upstream_resource"] = json!("https://api.example.com");
    config["token_endpoint_auth_method"] = json!("client_secret_post");
    let output = run(json!({"function": "grant", "configs": [config]}));
    let grant = &output["grants"][0];
    assert_eq!(
        grant["form"],
        json!([
            ["grant_type", "client_credentials"],
            ["client_id", "client"],
            ["client_secret", "secret"],
            ["scope", "read write"],
            ["audience", "api://aud"],
            ["resource", "https://api.example.com"],
        ])
    );
    assert_eq!(grant["headers"], json!([]));
    assert_eq!(grant["identity_key"].as_str().map(str::len), Some(64));
}

#[test]
fn grant_names_every_missing_field() {
    let output = run(json!({
        "function": "grant",
        "configs": [{"client_id": Value::Null, "client_secret": "s", "token_url": ""}],
    }));
    assert_eq!(
        output["grants"][0]["error"],
        json!("misconfigured: client_credentials config is missing: client_id, token_url")
    );
    assert_eq!(output["grants"][0]["identity_key"], Value::Null);
}

#[test]
fn mint_serves_the_cache_until_the_ttl_lapses_and_reports_the_denial() {
    let output = run(json!({
        "function": "mint",
        "config": config(),
        "outcomes": [
            {"kind": "success", "body": "{\"access_token\":\"tok-0\",\"expires_in\":3600}"},
            {"kind": "success", "body": "{\"access_token\":\"tok-1\",\"expires_in\":3600}"},
            {"kind": "denied", "status_code": 400_u16, "detail": "token endpoint returned HTTP 400"},
        ],
        "operations": [
            {"server_id": "s1"},
            {"server_id": "s1", "advance_ms": 1_000_i64},
            {"server_id": "s1", "advance_ms": 3_540_000_i64},
            {"server_id": "s2"},
        ],
    }));
    assert_eq!(output["post_calls"].as_array().map(Vec::len), Some(3));
    assert_eq!(
        output["results"][0]["token"]["expires_at_ms"],
        json!(4_600_000_i64)
    );
    assert_eq!(
        output["results"][1]["token"]["access_token"],
        json!("tok-0")
    );
    assert_eq!(
        output["results"][2]["token"]["access_token"],
        json!("tok-1")
    );
    assert_eq!(
        output["results"][3]["error"],
        json!(
            "misconfigured: OAuth2 client_credentials grant rejected: token endpoint returned HTTP 400"
        )
    );
}

#[test]
fn bearer_retries_a_401_once_and_keeps_the_second_answer() {
    let output = run(json!({
        "function": "bearer",
        "scenarios": [{
            "access_token": "old",
            "statuses": [401_u16, 401_u16],
            "refetch": ["fresh"],
        }],
    }));
    let scenario = &output["scenarios"][0];
    assert_eq!(
        scenario["sends"],
        json!([
            [["authorization", "Bearer old"]],
            [["authorization", "Bearer fresh"]],
        ])
    );
    assert_eq!(scenario["final_status"], json!(401_u16));
    assert_eq!(scenario["refetched"], json!(["old"]));
}
