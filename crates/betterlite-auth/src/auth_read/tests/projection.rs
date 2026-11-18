use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;
use rstest::rstest;

use crate::auth_read::adapter::AuthReadAdapter;
use crate::auth_read::membership::{
    SharedMembershipLoad, member_allowed_models, membership_from_shared_load,
};
use crate::auth_read::table::{AuthTable as _, FindOne, Presence};

use super::support::{Fields, Harness, Mode, recording_repo, recording_row_repo, rich_row};

fn membership(text: &str) -> LiteLLMTeamMembership {
    serde_json::from_str(text).expect("the membership fixture decodes")
}

fn filter() -> Fields {
    Fields::from([("id".to_owned(), "row-1".to_owned())])
}

#[rstest]
#[case::dictable(AuthReadAdapter::Dictable)]
#[case::model_dump(AuthReadAdapter::ModelDump)]
#[case::jwt_key_mapping(AuthReadAdapter::JwtKeyMapping)]
#[case::team(AuthReadAdapter::Team)]
#[case::vector_store(AuthReadAdapter::VectorStore)]
#[case::user(AuthReadAdapter::User)]
#[case::object_permission(AuthReadAdapter::ObjectPermission)]
#[case::tag(AuthReadAdapter::Tag)]
#[case::end_user(AuthReadAdapter::EndUser)]
#[case::model_access_group_budget(AuthReadAdapter::ModelAccessGroupBudget)]
#[tokio::test]
async fn structural_row_fields_retained(#[case] adapter: AuthReadAdapter) {
    let harness = Harness::new();
    let repo = recording_repo(Mode::Row);
    let expected = rich_row("user-1");
    let table = adapter.open(
        Arc::clone(&repo),
        "structural".to_owned(),
        harness.context(5.0),
    );
    let found = table
        .find_unique(FindOne {
            filter: filter(),
            include: Presence::Omitted,
        })
        .await
        .expect("the structural row read succeeds");
    assert_eq!(
        found,
        Some(expected),
        "{adapter:?} keeps every structural row field"
    );
    harness.drain().await;
}

#[tokio::test]
async fn nullable_user_memberships_retained() {
    let harness = Harness::new();
    let mut absent = rich_row("user-1");
    absent.organization_memberships = None;
    let present = rich_row("user-1");
    let absent_repo = recording_row_repo(Mode::Row, absent.clone());
    let absent_table = AuthReadAdapter::User.open(
        Arc::clone(&absent_repo),
        String::new(),
        harness.context(5.0),
    );
    let absent_found = absent_table
        .find_unique(FindOne {
            filter: filter(),
            include: Presence::Omitted,
        })
        .await
        .expect("the null membership read succeeds");
    assert_eq!(absent_found, Some(absent));
    let present_repo = recording_row_repo(Mode::Row, present.clone());
    let present_table = AuthReadAdapter::User.open(
        Arc::clone(&present_repo),
        String::new(),
        harness.context(5.0),
    );
    let present_found = present_table
        .find_unique(FindOne {
            filter: filter(),
            include: Presence::Omitted,
        })
        .await
        .expect("the membership sequence read succeeds");
    assert_eq!(present_found, Some(present));
    harness.drain().await;
}

#[test]
fn absent_budget_models_empty() {
    let membership = membership(r#"{"user_id":"user-1","team_id":"team-1"}"#);
    assert!(member_allowed_models(&membership).is_empty());
}

#[test]
fn null_or_empty_allowed_models_empty() {
    let null_budget =
        membership(r#"{"user_id":"user-1","team_id":"team-1","litellm_budget_table":null}"#);
    let null_models = membership(
        r#"{"user_id":"user-1","team_id":"team-1","litellm_budget_table":{"allowed_models":null}}"#,
    );
    let empty_models = membership(
        r#"{"user_id":"user-1","team_id":"team-1","litellm_budget_table":{"allowed_models":[]}}"#,
    );
    assert!(member_allowed_models(&null_budget).is_empty());
    assert!(member_allowed_models(&null_models).is_empty());
    assert!(member_allowed_models(&empty_models).is_empty());
}

#[test]
fn member_models_order_duplicates_and_empty_strings() {
    let membership = membership(
        r#"{"user_id":"user-1","team_id":"team-1","litellm_budget_table":{"allowed_models":["model-b","model-a","model-b",""]}}"#,
    );
    assert_eq!(
        member_allowed_models(&membership).to_vec(),
        [
            "model-b".to_owned(),
            "model-a".to_owned(),
            "model-b".to_owned(),
            String::new(),
        ]
    );
}

#[test]
fn typed_membership_passes() {
    let membership = membership(
        r#"{"user_id":"user-1","team_id":"team-1","litellm_budget_table":{"budget_id":"budget-1","allowed_models":["model-full"],"created_at":"2026-01-01T00:00:00"}}"#,
    );
    assert_eq!(
        member_allowed_models(&membership).to_vec(),
        ["model-full".to_owned()]
    );
    let narrowed = membership_from_shared_load(SharedMembershipLoad::Membership(Box::new(
        membership.clone(),
    )));
    assert_eq!(narrowed, Some(membership));
}

#[test]
fn matching_dict_not_shared_membership() {
    let payload = CacheValue::Object(BTreeMap::from([
        ("user_id".to_owned(), CacheValue::Text("user-1".to_owned())),
        ("team_id".to_owned(), CacheValue::Text("team-1".to_owned())),
        (
            "litellm_budget_table".to_owned(),
            CacheValue::Object(BTreeMap::from([(
                "allowed_models".to_owned(),
                CacheValue::Array(vec![CacheValue::Text("model-a".to_owned())]),
            )])),
        ),
    ]));
    assert_eq!(
        membership_from_shared_load(SharedMembershipLoad::Foreign(Box::new(payload))),
        None
    );
}

#[test]
fn narrowing_does_not_copy_or_validate() {
    let membership = membership(
        r#"{"user_id":"user-1","team_id":"team-1","spend":null,"litellm_budget_table":{"allowed_models":["","model-a","model-a"],"max_budget":-2.5}}"#,
    );
    let expected = membership.clone();
    let narrowed =
        membership_from_shared_load(SharedMembershipLoad::Membership(Box::new(membership)))
            .expect("the typed membership is accepted without a second validation");
    assert_eq!(narrowed, expected);
    assert_eq!(
        member_allowed_models(&narrowed).to_vec(),
        [String::new(), "model-a".to_owned(), "model-a".to_owned(),]
    );
    assert_eq!(
        membership_from_shared_load(SharedMembershipLoad::Foreign(Box::new(CacheValue::Array(
            Vec::new(),
        )))),
        None
    );
}
