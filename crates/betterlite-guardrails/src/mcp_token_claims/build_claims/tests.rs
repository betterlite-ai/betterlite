#![cfg(test)]

use betterlite_prompts::arguments::json::{Json, Object};
use reqwest::StatusCode;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::Number;

use crate::mcp_guardrail_translation::mapping_registration::McpCallType;
use crate::mcp_token_claims::build_claims::{ClaimRequest, build};
use crate::mcp_token_claims::{AuthFields, ClaimError, ClaimSettings, Claims};

const NOW: i64 = 1_700_000_000;
const DEFAULT_EXPIRY: i64 = 1_700_000_300;

fn claims(json: &str) -> Claims {
    serde_json::from_str(json).unwrap()
}

fn object(json: &str) -> Object {
    serde_json::from_str(json).unwrap()
}

fn auth() -> AuthFields {
    AuthFields {
        user_id: None,
        user_email: None,
        end_user_id: None,
        team_id: None,
        org_id: None,
        token: None,
        api_key: None,
    }
}

fn settings() -> ClaimSettings {
    ClaimSettings {
        issuer: "https://issuer.example".to_owned(),
        audience: "mcp".to_owned(),
        ttl_seconds: 300,
        allowed_scopes: None,
        add_claims: Object::new(),
        set_claims: Object::new(),
        remove_claims: Vec::new(),
        required_claims: Vec::new(),
        optional_claims: Vec::new(),
        end_user_claim_sources: vec!["token:sub".to_owned()],
    }
}

fn request_for<'input>(
    auth: &'input AuthFields,
    incoming: Option<&'input Claims>,
) -> ClaimRequest<'input> {
    ClaimRequest {
        auth,
        tool_name: None,
        incoming_claims: incoming,
        call_type: None,
    }
}

fn act(subject: &str) -> Json {
    Json::Object(Object::from_iter([(
        "sub".to_owned(),
        Json::Text(subject.to_owned()),
    )]))
}

#[test]
fn the_access_token_carries_the_standard_claims_and_identity() {
    let mut auth = auth();
    auth.user_id = Some("user-1".to_owned());
    auth.user_email = Some("alice@example.com".to_owned());
    auth.team_id = Some("team-1".to_owned());
    auth.end_user_id = Some("end-user-1".to_owned());
    let incoming = claims(r#"{"sub":"alice"}"#);
    let claims = build(&settings(), &request_for(&auth, Some(&incoming)), NOW).unwrap();
    assert_eq!(
        claims.get("iss"),
        Some(&Json::Text("https://issuer.example".to_owned()))
    );
    assert_eq!(claims.get("aud"), Some(&Json::Text("mcp".to_owned())));
    assert_eq!(claims.get("iat"), Some(&Json::Number(Number::from(NOW))));
    assert_eq!(
        claims.get("exp"),
        Some(&Json::Number(Number::from(DEFAULT_EXPIRY)))
    );
    assert_eq!(claims.get("nbf"), Some(&Json::Number(Number::from(NOW))));
    assert_eq!(claims.get("sub"), Some(&Json::Text("alice".to_owned())));
    assert_eq!(
        claims.get("email"),
        Some(&Json::Text("alice@example.com".to_owned()))
    );
    assert_eq!(
        claims.get("end_user_id"),
        Some(&Json::Text("end-user-1".to_owned()))
    );
    assert_eq!(claims.get("act"), Some(&act("team-1")));
    assert_eq!(
        claims.get("scope"),
        Some(&Json::Text("mcp:tools/list".to_owned()))
    );
}

#[rstest]
#[case::team(Some("team-1"), None, "team-1")]
#[case::org(None, Some("org-1"), "org-1")]
#[case::proxy(None, None, "litellm-proxy")]
fn the_actor_is_the_team_then_the_org_then_the_proxy(
    #[case] team: Option<&str>,
    #[case] org: Option<&str>,
    #[case] actor: &str,
) {
    let mut auth = auth();
    auth.team_id = team.map(str::to_owned);
    auth.org_id = org.map(str::to_owned);
    let claims = build(&settings(), &request_for(&auth, None), NOW).unwrap();
    assert_eq!(claims.get("act"), Some(&act(actor)));
}

#[test]
fn an_empty_team_id_falls_back_to_the_org() {
    let mut auth = auth();
    auth.team_id = Some(String::new());
    auth.org_id = Some("org-1".to_owned());
    let claims = build(&settings(), &request_for(&auth, None), NOW).unwrap();
    assert_eq!(claims.get("act"), Some(&act("org-1")));
}

#[test]
fn the_email_and_end_user_claims_are_left_out_when_unset() {
    let claims = build(&settings(), &request_for(&auth(), None), NOW).unwrap();
    assert_eq!(claims.get("email"), None);
    assert_eq!(claims.get("end_user_id"), None);
}

#[test]
fn an_empty_email_or_end_user_id_is_left_out() {
    let mut auth = auth();
    auth.user_email = Some(String::new());
    auth.end_user_id = Some(String::new());
    let claims = build(&settings(), &request_for(&auth, None), NOW).unwrap();
    assert_eq!(claims.get("email"), None);
    assert_eq!(claims.get("end_user_id"), None);
}

#[test]
fn a_tool_call_scopes_the_token_to_the_named_tool() {
    let auth = auth();
    let incoming = claims(r#"{"sub":"alice"}"#);
    let mut claim_request = request_for(&auth, Some(&incoming));
    claim_request.tool_name = Some("weather");
    claim_request.call_type = Some(McpCallType::CallMcpTool);
    let claims = build(&settings(), &claim_request, NOW).unwrap();
    assert_eq!(
        claims.get("scope"),
        Some(&Json::Text(
            "mcp:tools/call mcp:tools/weather:call".to_owned()
        ))
    );
}

#[test]
fn a_list_request_gets_the_list_scope() {
    let auth = auth();
    let mut claim_request = request_for(&auth, None);
    claim_request.call_type = Some(McpCallType::ListMcpTools);
    let claims = build(&settings(), &claim_request, NOW).unwrap();
    assert_eq!(
        claims.get("scope"),
        Some(&Json::Text("mcp:tools/list".to_owned()))
    );
}

#[test]
fn configured_scopes_replace_the_generated_scopes() {
    let mut settings = settings();
    settings.allowed_scopes = Some(vec![
        "mcp:tools/call".to_owned(),
        "mcp:tools/list".to_owned(),
    ]);
    let claims = build(&settings, &request_for(&auth(), None), NOW).unwrap();
    assert_eq!(
        claims.get("scope"),
        Some(&Json::Text("mcp:tools/call mcp:tools/list".to_owned()))
    );
}

#[test]
fn an_explicit_empty_scope_list_yields_an_empty_scope() {
    let mut settings = settings();
    settings.allowed_scopes = Some(Vec::new());
    let claims = build(&settings, &request_for(&auth(), None), NOW).unwrap();
    assert_eq!(claims.get("scope"), Some(&Json::Text(String::new())));
}

#[test]
fn the_expiry_is_the_issue_time_plus_the_configured_ttl() {
    let mut settings = settings();
    settings.ttl_seconds = 600;
    let claims = build(&settings, &request_for(&auth(), None), NOW).unwrap();
    assert_eq!(claims.get("iat"), Some(&Json::Number(Number::from(NOW))));
    assert_eq!(
        claims.get("exp"),
        Some(&Json::Number(Number::from(1_700_000_600_i64)))
    );
}

#[test]
fn optional_claims_pass_through_but_set_claims_still_override_them() {
    let incoming = claims(r#"{"sub":"alice","groups":["eng"]}"#);
    let mut settings = settings();
    settings.optional_claims = vec!["groups".to_owned()];
    settings.add_claims = object(r#"{"groups":["ignored"]}"#);
    settings.set_claims = object(r#"{"groups":["admin"]}"#);
    let claims = build(&settings, &request_for(&auth(), Some(&incoming)), NOW).unwrap();
    assert_eq!(
        claims.get("groups"),
        Some(&Json::List(vec![Json::Text("admin".to_owned())]))
    );
}

#[test]
fn claim_operations_run_last_and_override_every_computed_claim() {
    let mut settings = settings();
    settings.set_claims = object(r#"{"iss":"https://override.example","exp":42}"#);
    settings.remove_claims = vec!["nbf".to_owned()];
    let claims = build(&settings, &request_for(&auth(), None), NOW).unwrap();
    assert_eq!(
        claims.get("iss"),
        Some(&Json::Text("https://override.example".to_owned()))
    );
    assert_eq!(claims.get("exp"), Some(&Json::Number(Number::from(42_i32))));
    assert_eq!(claims.get("nbf"), None);
}

#[test]
fn add_claims_fill_keys_the_builder_does_not_compute() {
    let mut settings = settings();
    settings.add_claims = object(r#"{"deployment_id":"prod-001","sub":"ignored"}"#);
    let incoming = claims(r#"{"sub":"alice"}"#);
    let claims = build(&settings, &request_for(&auth(), Some(&incoming)), NOW).unwrap();
    assert_eq!(
        claims.get("deployment_id"),
        Some(&Json::Text("prod-001".to_owned()))
    );
    assert_eq!(claims.get("sub"), Some(&Json::Text("alice".to_owned())));
}

#[test]
fn the_subject_falls_back_to_the_api_key_hash() {
    let mut auth = auth();
    auth.token = Some(SecretString::from("sk-test-token"));
    let claims = build(&settings(), &request_for(&auth, None), NOW).unwrap();
    assert_eq!(
        claims.get("sub"),
        Some(&Json::Text("apikey:389faa3c31e10c88".to_owned()))
    );
}

#[test]
fn a_timestamp_overflow_is_a_typed_error() {
    let mut settings = settings();
    settings.ttl_seconds = 1;
    let error = build(&settings, &request_for(&auth(), None), i64::MAX).unwrap_err();
    assert_eq!(error, ClaimError::TimestampOverflow);
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
}
