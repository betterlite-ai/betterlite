#![cfg(test)]

use serde_json::{Value, json};

use betterlite_oracle_helper_api::helper::Helper as _;

fn input(value: Value) -> super::Input {
    serde_json::from_value(value).expect("the helper input")
}

#[test]
fn to_subject_prefers_the_org_then_the_team() {
    let answer = super::Op::run(&input(json!({
        "function": "to_subject",
        "principal": {"org_id": "org-1", "team_id": "team-1", "user_id": "user-1"},
        "subject_token": "inbound",
    })))
    .expect("an answer");
    assert_eq!(
        answer,
        json!({"tenant_id": "org-1", "subject_id": "user-1", "inbound_token": "inbound"})
    );
    let anonymous = super::Op::run(&input(json!({
        "function": "to_subject",
        "principal": null,
        "subject_token": null,
    })))
    .expect("an answer");
    assert_eq!(
        anonymous,
        json!({"tenant_id": "", "subject_id": "", "inbound_token": null})
    );
}

#[test]
fn static_header_spec_keeps_the_scheme_and_value() {
    let answer = super::Op::run(&input(json!({
        "function": "to_server_spec",
        "server": {
            "server_id": "srv-1",
            "name": "Server One",
            "transport": "http",
            "url": "https://mcp.example.com/mcp",
            "auth_type": "bearer_token",
            "authentication_token": "Bearer abc",
        },
    })))
    .expect("an answer");
    assert_eq!(answer["config"]["kind"], json!("api_key"));
    assert_eq!(
        answer["config"]["header"],
        json!({"name": "Authorization", "prefix": "Bearer"})
    );
    assert_eq!(
        answer["config"]["fields"]["key_source"]["value"],
        json!("abc")
    );
}

#[test]
fn a_deferred_mode_maps_to_null() {
    let answer = super::Op::run(&input(json!({
        "function": "to_server_spec",
        "server": {
            "server_id": "srv-1",
            "name": "Server One",
            "transport": "http",
            "url": "https://mcp.example.com/mcp",
            "auth_type": "aws_sigv4",
            "aws_region_name": "us-east-1",
        },
    })))
    .expect("an answer");
    assert_eq!(answer, Value::Null);
}

#[test]
fn protected_resource_path_inserts_the_scalar_prefix() {
    let answer = super::Op::run(&input(json!({
        "function": "protected_resource_path",
        "server": {
            "server_id": "srv-1",
            "name": "Server One",
            "transport": "http",
            "url": "https://mcp.example.com/mcp",
            "alias": "Alias",
        },
        "root_path": "/litellm",
        "scalar_root_path": "/litellm/",
    })))
    .expect("an answer");
    assert_eq!(
        answer,
        json!("/.well-known/oauth-protected-resource/litellm/mcp/Alias")
    );
}

#[test]
fn resolver_reports_a_url_credentials_error() {
    let answer = super::Op::run(&input(json!({
        "function": "credential_resolver",
        "server": {
            "server_id": "srv-1",
            "name": "Server One",
            "transport": "http",
            "url": "https://user:pass@mcp.example.com/mcp",
        },
        "subject": {"tenant_id": "org-1", "subject_id": "user-1", "inbound_token": null},
        "actions": ["resolve"],
    })))
    .expect("an answer");
    assert_eq!(
        answer["actions"][0]["tag"],
        json!("url_credentials_not_allowed")
    );
    assert_eq!(answer["token_endpoint_calls"], json!([]));
}
