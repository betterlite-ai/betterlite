use betterlite_db::identity_rows::user::LiteLLMUserTable;

use crate::auth_errors::conversion::AuthFailure;
use crate::auth_org_lookup::error::OrgLookupError;
use crate::auth_team_lookup::error::TeamLookupError;
use crate::auth_user_lookup::error::UserLookupError;
use crate::membership_load::error::MembershipLoadError;
use crate::model_grant_intersection::enforced::{EnforcedRequest, enforced_model_allowlists};
use crate::model_grant_intersection::error::IntersectionError;
use crate::model_grant_intersection::tests::support::{
    Effect, World, grant_token, list, membership_row, missing_user, owned, project_row, team_row,
    text, unavailable, user_read_failure, user_row,
};

#[tokio::test]
async fn no_db_two_lists() {
    let world = World::new();
    let token = grant_token(&[
        ("models", list(&["m1"])),
        ("team_models", list(&["gpt-4o"])),
    ]);
    let lists = enforced_model_allowlists(EnforcedRequest {
        token: &token,
        sources: None,
        now: 0.0,
    })
    .await
    .unwrap();
    assert_eq!(lists, vec![owned(&["m1"]), owned(&["gpt-4o"])]);
    assert!(world.effects().is_empty());

    let sentinel = grant_token(&[
        ("models", list(&["all-team-models"])),
        ("team_models", list(&["team-group"])),
        ("team_id", text("team-1")),
    ]);
    let sentinel_lists = enforced_model_allowlists(EnforcedRequest {
        token: &sentinel,
        sources: None,
        now: 0.0,
    })
    .await
    .unwrap();
    assert_eq!(
        sentinel_lists,
        vec![owned(&["team-group"]), owned(&["team-group"])]
    );
}

#[tokio::test]
async fn db_five_list_order() {
    let world = World::new()
        .with_user(Ok(Some(user_row("u1", &["user-group"]))))
        .with_project(Ok(Some(project_row("p1", &["project-group"]))));
    let token = grant_token(&[
        ("models", list(&["key-group"])),
        ("user_id", text("u1")),
        ("project_id", text("p1")),
    ]);
    let lists = enforced_model_allowlists(EnforcedRequest {
        token: &token,
        sources: Some(&world),
        now: 1.0,
    })
    .await
    .unwrap();
    assert_eq!(
        lists,
        vec![
            owned(&["key-group"]),
            Vec::new(),
            Vec::new(),
            owned(&["user-group"]),
            owned(&["project-group"]),
        ]
    );
    assert_eq!(
        world.effects(),
        vec![
            Effect::User("u1".to_owned()),
            Effect::Project("p1".to_owned()),
        ]
    );
}

#[tokio::test]
async fn team_suppresses_user_lookup() {
    let world = World::new()
        .with_team(Ok(team_row("team-1", &["team-group"], None)))
        .with_membership(Ok(Some(membership_row(
            "u1",
            "team-1",
            Some(&["member-group"]),
        ))));
    let token = grant_token(&[
        ("models", list(&["key-group"])),
        ("team_id", text("team-1")),
        ("user_id", text("u1")),
    ]);
    let lists = enforced_model_allowlists(EnforcedRequest {
        token: &token,
        sources: Some(&world),
        now: 0.0,
    })
    .await
    .unwrap();
    assert_eq!(
        lists,
        vec![
            owned(&["key-group"]),
            owned(&["team-group"]),
            owned(&["member-group"]),
            Vec::new(),
            Vec::new(),
        ]
    );
    assert_eq!(
        world.effects(),
        vec![
            Effect::Team("team-1".to_owned()),
            Effect::Membership {
                user_id: "u1".to_owned(),
                team_id: "team-1".to_owned(),
            },
        ]
    );
}

async fn assert_user_failure_escapes(answer: UserLookupError) {
    let token = grant_token(&[("user_id", text("u1"))]);
    let world = World::new().with_user(Err(answer));
    let outcome = enforced_model_allowlists(EnforcedRequest {
        token: &token,
        sources: Some(&world),
        now: 0.0,
    })
    .await;
    assert!(matches!(outcome, Err(IntersectionError::User(_))));
}

#[tokio::test]
async fn user_not_found_only_suppressed() {
    let token = grant_token(&[("user_id", text("u1"))]);
    let world = World::new().with_user(Err(missing_user()));
    let lists = enforced_model_allowlists(EnforcedRequest {
        token: &token,
        sources: Some(&world),
        now: 0.0,
    })
    .await
    .unwrap();
    assert_eq!(lists[3], Vec::<String>::new());

    assert_user_failure_escapes(UserLookupError::NoDatabase).await;
    assert_user_failure_escapes(user_read_failure()).await;
    assert_user_failure_escapes(UserLookupError::Unavailable(unavailable())).await;
    assert_user_failure_escapes(UserLookupError::PreLookup(unavailable())).await;
}

// LiteLLM declares list[str]; a stored non-text entry grants nothing because no allowlist check matches it
#[tokio::test]
async fn non_text_user_models_grant_nothing() {
    let user: LiteLLMUserTable =
        serde_json::from_str(r#"{"user_id":"u1","models":["user-group",7]}"#)
            .expect("the user row fixture parses");
    let world = World::new().with_user(Ok(Some(user)));
    let token = grant_token(&[("user_id", text("u1"))]);
    let lists = enforced_model_allowlists(EnforcedRequest {
        token: &token,
        sources: Some(&world),
        now: 0.0,
    })
    .await
    .unwrap();
    assert_eq!(lists[3], owned(&["user-group"]));
}

async fn assert_team_failure_escapes() {
    let token = grant_token(&[("team_id", text("team-1"))]);
    let world = World::new().with_team(Err(TeamLookupError::NoDatabase));
    let error = enforced_model_allowlists(EnforcedRequest {
        token: &token,
        sources: Some(&world),
        now: 0.0,
    })
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        IntersectionError::Team(TeamLookupError::NoDatabase)
    ));
    assert_eq!(error.http_status(), None);
    assert!(matches!(error.as_auth_failure(), AuthFailure::Other { .. }));

    let missing_world = World::new().with_team(Err(TeamLookupError::TeamNotFound {
        team_id: "team-1".to_owned(),
    }));
    let not_found = enforced_model_allowlists(EnforcedRequest {
        token: &token,
        sources: Some(&missing_world),
        now: 0.0,
    })
    .await
    .unwrap_err();
    assert_eq!(not_found.http_status(), Some(404));
}

#[tokio::test]
async fn team_project_errors_escape() {
    assert_team_failure_escapes().await;
    let token = grant_token(&[("project_id", text("p1"))]);
    let world = World::new().with_project(Err(OrgLookupError::NoDatabase));
    let error = enforced_model_allowlists(EnforcedRequest {
        token: &token,
        sources: Some(&world),
        now: 0.0,
    })
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        IntersectionError::Project(OrgLookupError::NoDatabase)
    ));
}

#[tokio::test]
async fn membership_error_escapes() {
    let token = grant_token(&[("team_id", text("team-1")), ("user_id", text("u1"))]);
    let world = World::new()
        .with_team(Ok(team_row("team-1", &[], None)))
        .with_membership(Err(MembershipLoadError::NoDatabase));
    let error = enforced_model_allowlists(EnforcedRequest {
        token: &token,
        sources: Some(&world),
        now: 0.0,
    })
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        IntersectionError::Membership(MembershipLoadError::NoDatabase)
    ));
}
