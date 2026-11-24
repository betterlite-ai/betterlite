use std::sync::Mutex;

use betterlite_db::team_rows::team::LiteLLMTeamTable;

use super::support::{FakeTeams, strings, team, team_caller, user_row};
use crate::key_list_filters::error::{KeyListFiltersError, KeyListReadError};
use crate::key_list_filters::teams::{
    admin_team_ids, fetch_user_team_objects, list_permission_team_ids, member_team_ids,
};

const ADMIN_T1: &str = r#"{"team_id":"t1","members_with_roles":[{"user_id":"u1","role":"admin"}],"team_member_permissions":["/key/list"]}"#;
const MEMBER_T2: &str = r#"{"team_id":"t2","members_with_roles":[{"user_id":"u1","role":"user"}]}"#;
const PERMITTED_T3: &str = r#"{"team_id":"t3","members_with_roles":[{"user_id":"u1","role":"user"}],"team_member_permissions":["/key/list"]}"#;
const OTHER_ROUTE_T4: &str = r#"{"team_id":"t4","members_with_roles":[{"user_id":"u1","role":"user"}],"team_member_permissions":["/key/info"]}"#;
const EMAIL_ONLY_T5: &str =
    r#"{"team_id":"t5","members_with_roles":[{"user_email":"e@example.com","role":"user"}]}"#;
const FOREIGN_T6: &str =
    r#"{"team_id":"t6","members_with_roles":[{"user_id":"u2","role":"user"}]}"#;

fn team_ids(teams: &[LiteLLMTeamTable]) -> Vec<String> {
    teams
        .iter()
        .map(|found| found.fields.base.team_id.clone())
        .collect()
}

#[tokio::test]
async fn teams_empty_no_query() {
    let fake = FakeTeams::default();
    assert!(
        fetch_user_team_objects(None, &fake)
            .await
            .unwrap()
            .is_empty()
    );
    let user = user_row(r#"{"user_id":"u1","teams":[]}"#);
    assert!(
        fetch_user_team_objects(Some(&user), &fake)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(fake.reads().is_empty());
}

#[tokio::test]
async fn teams_single_query() {
    let fake = FakeTeams::with_records(&[MEMBER_T2, ADMIN_T1]);
    let user = user_row(r#"{"user_id":"u1","teams":["t1","t2"]}"#);
    let teams = fetch_user_team_objects(Some(&user), &fake).await.unwrap();
    assert_eq!(team_ids(&teams), strings(&["t2", "t1"]));
    assert_eq!(fake.reads(), vec![strings(&["t1", "t2"])]);
}

#[tokio::test]
async fn teams_invalid_row_propagates() {
    let fake = FakeTeams::with_records(&[r#"{"members_with_roles":"not-json"}"#, ADMIN_T1]);
    let user = user_row(r#"{"user_id":"u1","teams":["t1"]}"#);
    let outcome = fetch_user_team_objects(Some(&user), &fake).await;
    assert!(matches!(
        outcome,
        Err(KeyListFiltersError::Read(KeyListReadError::Team(_)))
    ));
    let missing_id = FakeTeams::with_records(&[r#"{"members_with_roles":[]}"#]);
    let missing_outcome = fetch_user_team_objects(Some(&user), &missing_id).await;
    assert!(matches!(
        missing_outcome,
        Err(KeyListFiltersError::Read(KeyListReadError::Team(_)))
    ));
}

#[tokio::test]
async fn teams_query_failure_propagates() {
    let fake = FakeTeams {
        records: Vec::new(),
        fail: true,
        reads: Mutex::new(Vec::new()),
    };
    let user = user_row(r#"{"user_id":"u1","teams":["t1"]}"#);
    let outcome = fetch_user_team_objects(Some(&user), &fake).await;
    assert!(matches!(
        outcome,
        Err(KeyListFiltersError::Read(KeyListReadError::Teams(_)))
    ));
}

#[tokio::test]
async fn teams_admin_ids() {
    let caller = team_caller(Some("u1"));
    let teams = [team(ADMIN_T1), team(MEMBER_T2), team(ADMIN_T1)];
    assert_eq!(admin_team_ids(&caller, &teams), strings(&["t1", "t1"]));
}

#[tokio::test]
async fn teams_list_permission_nonadmin() {
    let caller = team_caller(Some("u1"));
    let teams = [
        team(ADMIN_T1),
        team(PERMITTED_T3),
        team(MEMBER_T2),
        team(OTHER_ROUTE_T4),
    ];
    assert_eq!(admin_team_ids(&caller, &teams), strings(&["t1"]));
    assert_eq!(list_permission_team_ids(&caller, &teams), strings(&["t3"]));
    assert_eq!(
        member_team_ids(&caller, &teams),
        strings(&["t1", "t3", "t2", "t4"])
    );
}

#[tokio::test]
async fn teams_member_null_id_excluded() {
    let caller = team_caller(Some("u1"));
    let teams = [team(EMAIL_ONLY_T5), team(FOREIGN_T6)];
    assert!(member_team_ids(&caller, &teams).is_empty());
    let anonymous = team_caller(None);
    let email_only = [team(EMAIL_ONLY_T5)];
    assert!(member_team_ids(&anonymous, &email_only).is_empty());
}
