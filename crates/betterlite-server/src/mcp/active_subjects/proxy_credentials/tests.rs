#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_auth::session_token_codec::record::SessionRecord;
use betterlite_auth::user_roles::UserRole;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_mcp::gateway_dcr::ports::{
    ConsentTeam, LookupConsentTeams, MintProxyCredential, ProxyCredentialMintFailure,
    ReloadUserFailure,
};
use chrono::TimeDelta;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::super::tests::{
    FakeStore, UserAnswer, detail, fixed_now, session_codec, subjects, user_row,
};
use super::super::{SubjectReadError, UserRowSource};
use crate::cli_team_grants::records::CliSsoTeamDetail;

const DAY_SECONDS: u64 = 86_400;

fn teamless(alias: &str) -> CliSsoTeamDetail {
    CliSsoTeamDetail {
        team_id: None,
        team_alias: Some(alias.to_owned()),
        team_models: Vec::new(),
        team_model_aliases: None,
    }
}

fn token_record(credential_key: &str) -> SessionRecord {
    let auth = session_codec()
        .session_from_token(credential_key)
        .unwrap()
        .unwrap();
    SessionRecord::from_auth(&auth)
}

fn role(text: &str) -> Option<UserRole> {
    Some(text.parse().unwrap())
}

#[tokio::test]
async fn consent_reads_the_cache_source() {
    let store = FakeStore {
        user: UserAnswer::Found(user_row(r#"{"user_id":"user-1"}"#)),
        details: Some(Vec::new()),
        ..FakeStore::new()
    };
    let calls = store.calls();
    let active = subjects(store);
    let teams = active.consent_teams("user-1").await.unwrap();
    assert!(teams.is_empty());
    let calls = calls.lock().unwrap();
    assert_eq!(
        calls.users,
        vec![("user-1".to_owned(), UserRowSource::Cache)]
    );
    assert_eq!(calls.teams, vec![Vec::<String>::new()]);
}

#[rstest]
#[case::missing(UserAnswer::Missing, ReloadUserFailure::NoActiveKey)]
#[case::unavailable(
    UserAnswer::Failed(SubjectReadError::Unavailable),
    ReloadUserFailure::Unavailable
)]
#[tokio::test]
async fn consent_returns_the_user_failure(
    #[case] answer: UserAnswer,
    #[case] expected: ReloadUserFailure,
) {
    let store = FakeStore {
        user: answer,
        ..FakeStore::new()
    };
    let active = subjects(store);
    let outcome = active.consent_teams("user-1").await;
    assert_eq!(outcome.unwrap_err(), expected);
}

#[tokio::test]
async fn consent_unknown_team_details_is_unavailable() {
    let store = FakeStore {
        user: UserAnswer::Found(user_row(r#"{"user_id":"user-1","teams":["team-a"]}"#)),
        details: None,
        ..FakeStore::new()
    };
    let calls = store.calls();
    let active = subjects(store);
    let outcome = active.consent_teams("user-1").await;
    assert_eq!(outcome.unwrap_err(), ReloadUserFailure::Unavailable);
    assert_eq!(calls.lock().unwrap().teams, vec![vec!["team-a".to_owned()]]);
}

#[tokio::test]
async fn consent_drops_details_without_a_team_id() {
    let store = FakeStore {
        user: UserAnswer::Found(user_row(r#"{"user_id":"user-1","teams":["team-a"]}"#)),
        details: Some(vec![
            detail("team-a", "alias-a", &[]),
            teamless("orphan"),
            detail("team-b", "alias-b", &[]),
        ]),
        ..FakeStore::new()
    };
    let active = subjects(store);
    let teams = active.consent_teams("user-1").await.unwrap();
    assert_eq!(
        teams,
        vec![
            ConsentTeam {
                team_id: "team-a".to_owned(),
                team_alias: Some("alias-a".to_owned()),
            },
            ConsentTeam {
                team_id: "team-b".to_owned(),
                team_alias: Some("alias-b".to_owned()),
            },
        ]
    );
}

#[tokio::test]
async fn mint_reads_the_database_source_and_returns_the_user_failure() {
    let store = FakeStore {
        user: UserAnswer::Failed(SubjectReadError::Unavailable),
        ..FakeStore::new()
    };
    let calls = store.calls();
    let active = subjects(store);
    let outcome = active.mint("user-1", None).await;
    assert_eq!(
        outcome.unwrap_err(),
        ProxyCredentialMintFailure::Unavailable
    );
    assert_eq!(
        calls.lock().unwrap().users,
        vec![("user-1".to_owned(), UserRowSource::Database)]
    );
}

#[tokio::test]
async fn mint_refuses_a_team_the_user_is_not_in_before_any_team_read() {
    let store = FakeStore {
        user: UserAnswer::Found(user_row(r#"{"user_id":"user-1","teams":["team-a"]}"#)),
        details: Some(vec![detail("team-a", "alias-a", &[])]),
        ..FakeStore::new()
    };
    let calls = store.calls();
    let active = subjects(store);
    let outcome = active.mint("user-1", Some("team-b")).await;
    assert_eq!(outcome.unwrap_err(), ProxyCredentialMintFailure::NotAMember);
    assert!(calls.lock().unwrap().teams.is_empty());
}

#[tokio::test]
async fn mint_without_team_details_is_unavailable() {
    let store = FakeStore {
        user: UserAnswer::Found(user_row(r#"{"user_id":"user-1","teams":["team-a"]}"#)),
        details: None,
        ..FakeStore::new()
    };
    let active = subjects(store);
    let outcome = active.mint("user-1", None).await;
    assert_eq!(
        outcome.unwrap_err(),
        ProxyCredentialMintFailure::Unavailable
    );
}

#[tokio::test]
async fn mint_without_a_team_for_live_teams_is_team_required() {
    let store = FakeStore {
        user: UserAnswer::Found(user_row(r#"{"user_id":"user-1","teams":["team-a"]}"#)),
        details: Some(vec![detail("team-a", "alias-a", &[])]),
        ..FakeStore::new()
    };
    let active = subjects(store);
    let outcome = active.mint("user-1", None).await;
    assert_eq!(
        outcome.unwrap_err(),
        ProxyCredentialMintFailure::TeamRequired
    );
}

#[tokio::test]
async fn mint_refuses_a_member_whose_team_row_is_gone() {
    let store = FakeStore {
        user: UserAnswer::Found(user_row(r#"{"user_id":"user-1","teams":["team-a"]}"#)),
        details: Some(Vec::new()),
        ..FakeStore::new()
    };
    let active = subjects(store);
    let outcome = active.mint("user-1", Some("team-a")).await;
    assert_eq!(outcome.unwrap_err(), ProxyCredentialMintFailure::NotAMember);
}

#[tokio::test]
async fn mint_with_no_team_for_a_user_whose_teams_are_gone() {
    let store = FakeStore {
        user: UserAnswer::Found(user_row(r#"{"user_id":"user-1","teams":["gone"]}"#)),
        details: Some(Vec::new()),
        ..FakeStore::new()
    };
    let active = subjects(store);
    let credential = active.mint("user-1", None).await.unwrap();
    assert_eq!(credential.team_id, None);
    assert_eq!(token_record(credential.key.expose_secret()).team_id, None);
}

#[tokio::test]
async fn mint_without_teams_never_reads_team_details() {
    let store = FakeStore {
        user: UserAnswer::Found(user_row(r#"{"user_id":"user-1","models":["gpt-4o"]}"#)),
        details: None,
        ..FakeStore::new()
    };
    let calls = store.calls();
    let active = subjects(store);
    let credential = active.mint("user-1", None).await.unwrap();
    assert!(calls.lock().unwrap().teams.is_empty());
    assert_eq!(
        token_record(credential.key.expose_secret()).models,
        vec![JsonData::String("gpt-4o".to_owned())]
    );
}

#[rstest]
#[case::missing(UserAnswer::Missing, ProxyCredentialMintFailure::NoActiveKey)]
#[case::scim_deactivated(
    UserAnswer::Found(user_row(r#"{"user_id":"user-1","metadata":{"scim_active":false}}"#)),
    ProxyCredentialMintFailure::NoActiveKey
)]
#[tokio::test]
async fn mint_refuses_a_user_that_is_not_live(
    #[case] answer: UserAnswer,
    #[case] expected: ProxyCredentialMintFailure,
) {
    let store = FakeStore {
        user: answer,
        ..FakeStore::new()
    };
    let active = subjects(store);
    let outcome = active.mint("user-1", None).await;
    assert_eq!(outcome.unwrap_err(), expected);
}

#[tokio::test]
async fn mint_token_carries_the_grant_team_and_its_scope() {
    let scoped = CliSsoTeamDetail {
        team_id: Some("team-a".to_owned()),
        team_alias: Some("alias-a".to_owned()),
        team_models: vec!["gpt-4o-mini".to_owned()],
        team_model_aliases: Some(BTreeMap::from([("gpt-4o".to_owned(), "smart".to_owned())])),
    };
    let store = FakeStore {
        user: UserAnswer::Found(user_row(
            r#"{"user_id":"user-1","user_role":"proxy_admin","models":["gpt-4o"],"teams":["team-a"]}"#,
        )),
        details: Some(vec![scoped]),
        ..FakeStore::new()
    };
    let active = subjects(store);
    let credential = active.mint("user-1", Some("team-a")).await.unwrap();
    assert_eq!(credential.expires_in.get(), DAY_SECONDS);
    assert_eq!(credential.user_id, "user-1");
    assert_eq!(credential.team_id.as_deref(), Some("team-a"));
    let record = token_record(credential.key.expose_secret());
    assert_eq!(record.user_id.as_deref(), Some("user-1"));
    assert_eq!(record.team_id.as_deref(), Some("team-a"));
    assert_eq!(record.team_alias.as_deref(), Some("alias-a"));
    assert_eq!(
        record.team_models,
        vec![JsonData::String("gpt-4o-mini".to_owned())]
    );
    assert_eq!(
        record.team_model_aliases,
        Some(BTreeMap::from([(
            "gpt-4o".to_owned(),
            JsonData::String("smart".to_owned())
        )]))
    );
    assert!(record.models.is_empty());
    assert_eq!(record.user_role, role("proxy_admin"));
    assert!(record.is_session_token);
}

#[tokio::test]
async fn mint_token_without_a_team_carries_the_user_models_and_effective_role() {
    let store = FakeStore {
        user: UserAnswer::Found(user_row(r#"{"user_id":"user-1","models":["gpt-4o"]}"#)),
        details: Some(Vec::new()),
        ..FakeStore::new()
    };
    let active = subjects(store);
    let credential = active.mint("user-1", None).await.unwrap();
    assert_eq!(credential.expires_in.get(), DAY_SECONDS);
    assert_eq!(credential.team_id, None);
    let record = token_record(credential.key.expose_secret());
    assert_eq!(record.models, vec![JsonData::String("gpt-4o".to_owned())]);
    assert_eq!(record.team_id, None);
    assert_eq!(record.user_role, role("internal_user"));
    assert!(record.team_models.is_empty());
}

#[tokio::test]
async fn mint_token_uses_the_configured_lifetime() {
    let store = FakeStore {
        user: UserAnswer::Found(user_row(r#"{"user_id":"user-1"}"#)),
        details: Some(Vec::new()),
        ..FakeStore::new()
    };
    let mut active = subjects(store);
    active.settings.cli_token_hours = 2;
    let credential = active.mint("user-1", None).await.unwrap();
    assert_eq!(credential.expires_in.get(), 7_200);
    let expected = fixed_now()
        .checked_add_signed(TimeDelta::hours(2))
        .unwrap()
        .format("%Y-%m-%dT%H:%M:%S%.3f+00:00")
        .to_string();
    assert_eq!(
        token_record(credential.key.expose_secret()).expires,
        Some(expected)
    );
}
