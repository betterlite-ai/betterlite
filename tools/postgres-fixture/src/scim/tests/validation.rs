use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::dashboard::payload::JsonPayload;
use crate::error::FixtureError;
use crate::scim::model::{MemberRole, ScimKeyRow, ScimMember};
use crate::seed::{CallerRole, DatabaseFixture};

use super::support::{SAMPLE, decode};

#[test]
fn key_rows_compare_token_blocked_and_metadata() {
    let base = decode_key(r#"{"token":"sk-a","blocked":true,"metadata":{"scim_blocked":true}}"#);
    assert_eq!(
        base,
        decode_key(r#"{"token":"sk-a","blocked":true,"metadata":{"scim_blocked":true}}"#)
    );
    assert_ne!(
        base,
        decode_key(r#"{"token":"sk-b","blocked":true,"metadata":{"scim_blocked":true}}"#)
    );
    assert_ne!(
        base,
        decode_key(r#"{"token":"sk-a","blocked":false,"metadata":{"scim_blocked":true}}"#)
    );
    assert_ne!(
        base,
        decode_key(r#"{"token":"sk-a","blocked":true,"metadata":{"scim_blocked":false}}"#)
    );
}

fn decode_key(document: &str) -> ScimKeyRow {
    serde_json::from_str(document).expect("the key case decodes")
}

#[test]
fn decodes_the_scim_seed_typed_fields() {
    let seed = decode(SAMPLE);
    seed.validate().expect("the sample seed validates");
    let scim = seed.scim.as_ref().expect("the seed carries scim");
    let alice = &scim.users[0];
    assert_eq!(alice.user_id, "alice");
    assert_eq!(
        alice
            .created_at
            .as_ref()
            .map(ToString::to_string)
            .as_deref(),
        Some("2025-01-01 00:00:00")
    );
    assert_eq!(alice.user_alias.as_deref(), Some("Alice"));
    assert_eq!(alice.sso_user_id.as_deref(), Some(""));
    assert_eq!(alice.user_role, Some(CallerRole::ProxyAdmin));
    let teams = alice.teams.as_ref().expect("alice carries teams");
    assert_eq!(teams, &["team-red".to_owned(), "team-unseeded".to_owned()]);
    let metadata = alice.metadata.as_ref().expect("alice carries metadata");
    assert_eq!(
        metadata.get("origin"),
        Some(&JsonPayload::Text("scim".to_owned()))
    );
    assert_eq!(metadata.get("scim_managed"), Some(&JsonPayload::Bool(true)));
    let first = scim.teams[0]
        .members_with_roles
        .first()
        .expect("the roster has a member");
    assert_eq!(
        first,
        &ScimMember {
            user_id: Some("alice".to_owned()),
            user_email: None,
            role: MemberRole::Admin,
        }
    );
    assert_eq!(scim.teams[0].members_with_roles[1].role, MemberRole::User);
    assert!(scim.teams[1].members_with_roles.is_empty());
    assert_eq!(scim.keys[0].blocked, Some(true));
    assert_eq!(scim.keys[0].token.expose_secret(), "sk-alice-token");
    assert_eq!(scim.keys[1].blocked, Some(false));
    assert_eq!(scim.keys[2].blocked, None);
}

#[rstest]
#[case::missing_user(
    r#"{"users":[{"user_id":"alice"}],"scim":{"users":[{"user_id":"carol"}]}}"#,
    "scim user row 0 references missing user carol"
)]
#[case::duplicate_user(
    r#"{"users":[{"user_id":"alice"}],"scim":{"users":[{"user_id":"alice"},{"user_id":"alice"}]}}"#,
    "scim user row 1 repeats an earlier user"
)]
#[case::conflicting_user_role(
    r#"{"users":[{"user_id":"alice"}],"keys":[{"token":"sk-alice-token","user_id":"alice","role":"proxy_admin"}],"scim":{"users":[{"user_id":"alice","user_role":"internal_user"}]}}"#,
    "scim user row 0 user_role disagrees with the role its key rows give user alice"
)]
#[case::missing_team(
    r#"{"teams":[{"team_id":"team-red"}],"scim":{"teams":[{"team_id":"team-blue"}]}}"#,
    "scim team row 0 references missing team team-blue"
)]
#[case::duplicate_team(
    r#"{"teams":[{"team_id":"team-red"}],"scim":{"teams":[{"team_id":"team-red"},{"team_id":"team-red"}]}}"#,
    "scim team row 1 repeats an earlier team"
)]
#[case::unknown_key(
    r#"{"users":[{"user_id":"alice"}],"keys":[{"token":"sk-alice-token","user_id":"alice"}],"scim":{"keys":[{"token":"sk-other"}]}}"#,
    "scim key row 0 references no fixture key"
)]
#[case::duplicate_key(
    r#"{"users":[{"user_id":"alice"}],"keys":[{"token":"sk-alice-token","user_id":"alice"}],"scim":{"keys":[{"token":"sk-alice-token"},{"token":"sk-alice-token"}]}}"#,
    "scim key row 1 repeats an earlier key"
)]
fn rejects_bad_overlays(#[case] document: &str, #[case] message: &str) {
    let seed = decode(document);
    let error = seed.validate().expect_err("the seed is rejected");
    let FixtureError::Scim(source) = error else {
        panic!("expected a scim fixture error, got {error:?}");
    };
    assert_eq!(source.to_string(), message);
}

#[test]
fn accepts_a_user_role_its_keys_agree_with() {
    let seed = decode(
        r#"{"users":[{"user_id":"alice"}],"keys":[{"token":"sk-alice-token","user_id":"alice","role":"internal_user"}],"scim":{"users":[{"user_id":"alice","user_role":"internal_user"}]}}"#,
    );
    seed.validate().expect("the roles agree");
}

#[test]
fn denies_unknown_scim_fields() {
    let document = r#"{"seed":{"users":[{"user_id":"alice"}],"scim":{"users":[{"user_id":"alice","teams_count":2}]}}}"#;
    assert!(serde_json::from_str::<DatabaseFixture>(document).is_err());
}
