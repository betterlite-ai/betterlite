use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};

use crate::credentials::contracts::{
    CliAuthResult, CliContextObj, CliPollData, CliSsoStartData, CliTeam, CliTokenData,
};
use crate::token_records::record::CliTokenRecord;

#[test]
fn poll_data_reads_every_declared_field() {
    let body = r#"{"status":"ready","key":"sk-poll","user_id":"user-7","teams":["team-1","team-2"],"team_details":[{"team_id":"team-1"}],"requires_team_selection":true,"team_id":"team-2"}"#;
    let data: CliPollData = serde_json::from_str(body).expect("parses the poll data");

    assert_eq!(data.status.as_deref(), Some("ready"));
    assert_eq!(data.key.as_ref().expect("a key").expose_secret(), "sk-poll");
    assert_eq!(data.user_id.as_deref(), Some("user-7"));
    assert_eq!(data.teams, vec!["team-1", "team-2"]);
    assert_eq!(
        data.team_details.expect("team details").get(),
        r#"[{"team_id":"team-1"}]"#
    );
    assert!(data.requires_team_selection);
    assert_eq!(data.team_id.as_deref(), Some("team-2"));
}

#[test]
fn poll_data_defaults_every_absent_field() {
    let data: CliPollData = serde_json::from_str("{}").expect("parses an empty poll data");

    assert!(data.status.is_none());
    assert!(data.key.is_none());
    assert!(data.user_id.is_none());
    assert!(data.teams.is_empty());
    assert!(data.team_details.is_none());
    assert!(!data.requires_team_selection);
    assert!(data.team_id.is_none());
}

#[test]
fn sso_start_data_requires_the_three_codes_and_defaults_the_uri() {
    let data: CliSsoStartData =
        serde_json::from_str(r#"{"login_id":"login-1","poll_secret":"poll-1","user_code":"ABCD"}"#)
            .expect("parses the start data");
    assert_eq!(data.login_id, "login-1");
    assert_eq!(data.poll_secret.expose_secret(), "poll-1");
    assert_eq!(data.user_code, "ABCD");
    assert!(data.verification_uri_complete.is_none());

    let with_uri: CliSsoStartData = serde_json::from_str(
        r#"{"login_id":"login-1","poll_secret":"poll-1","user_code":"ABCD","verification_uri_complete":"https://gateway.example/verify"}"#,
    )
    .expect("parses the start data with a URI");
    assert_eq!(
        with_uri.verification_uri_complete.as_deref(),
        Some("https://gateway.example/verify")
    );
}

#[test]
fn sso_start_data_names_a_missing_required_field() {
    let error =
        serde_json::from_str::<CliSsoStartData>(r#"{"poll_secret":"poll-1","user_code":"ABCD"}"#)
            .expect_err("rejects a missing login id");

    assert!(error.to_string().contains("login_id"));
}

#[test]
fn team_records_carry_aliases_models_and_a_decimal_budget() {
    let team = CliTeam {
        team_id: Some(String::from("team-1")),
        team_alias: Some(String::from("Alpha")),
        models: vec![String::from("gpt-4o")],
        max_budget: Some(Decimal::new(10_050, 2)),
    };

    assert_eq!(
        team.max_budget.map(|budget| budget.to_string()),
        Some(String::from("100.50"))
    );
    assert!(CliTeam::default().max_budget.is_none());
}

#[test]
fn auth_result_carries_the_secret_key_and_optional_identity() {
    let result = CliAuthResult {
        api_key: SecretString::from("sk-auth".to_owned()),
        user_id: None,
        teams: vec![String::from("team-1")],
        team_id: None,
    };

    assert_eq!(result.api_key.expose_secret(), "sk-auth");
    assert!(result.user_id.is_none());
    assert_eq!(result.teams, vec!["team-1"]);
}

#[test]
fn context_and_token_aliases_name_the_declared_shapes() {
    let context: CliContextObj = CliContextObj::default();
    let token: CliTokenData = CliTokenRecord::default();

    assert_eq!(context.base_url, "");
    assert!(!context.base_url_explicit);
    assert_eq!(token.base_url, "");
    assert!(token.key.is_none());
}
