use crate::model_grant_intersection::member::team_member_granted_models;
use crate::model_grant_intersection::tests::support::{
    World, grant_token, membership_row, owned, team_row, text,
};

async fn no_team_returns_empty() {
    let world = World::new();
    let token = grant_token(&[("user_id", text("u1"))]);
    let models = team_member_granted_models(&token, None, None, false, &world)
        .await
        .unwrap();
    assert_eq!(models, Vec::<String>::new());
    assert!(world.effects().is_empty());
}

async fn no_user_returns_empty() {
    let world = World::new();
    let team = team_row("team-1", &[], None);
    let token = grant_token(&[]);
    let models = team_member_granted_models(&token, Some(&team), None, false, &world)
        .await
        .unwrap();
    assert_eq!(models, Vec::<String>::new());
    assert!(world.effects().is_empty());
}

async fn missing_row_returns_empty() {
    let world = World::new().with_membership(Ok(None));
    let token = grant_token(&[("user_id", text("u1"))]);
    let team = team_row("team-1", &[], None);
    let models = team_member_granted_models(&token, Some(&team), None, false, &world)
        .await
        .unwrap();
    assert_eq!(models, Vec::<String>::new());
}

async fn empty_allowed_models_returns_empty() {
    let world = World::new().with_membership(Ok(Some(membership_row("u1", "team-1", None))));
    let token = grant_token(&[("user_id", text("u1"))]);
    let team = team_row("team-1", &[], None);
    let models = team_member_granted_models(&token, Some(&team), None, false, &world)
        .await
        .unwrap();
    assert_eq!(models, Vec::<String>::new());
}

async fn granted_member_models_are_returned() {
    let world = World::new().with_membership(Ok(Some(membership_row(
        "u1",
        "team-1",
        Some(&["member-group"]),
    ))));
    let token = grant_token(&[("user_id", text("u1"))]);
    let team = team_row("team-1", &[], None);
    let models = team_member_granted_models(&token, Some(&team), None, false, &world)
        .await
        .unwrap();
    assert_eq!(models, owned(&["member-group"]));
}

#[tokio::test]
async fn missing_member_inputs_empty() {
    no_team_returns_empty().await;
    no_user_returns_empty().await;
    missing_row_returns_empty().await;
    empty_allowed_models_returns_empty().await;
    granted_member_models_are_returned().await;
}

#[tokio::test]
async fn membership_loaded_none_no_query() {
    let world = World::new();
    let token = grant_token(&[("user_id", text("u1"))]);
    let team = team_row("team-1", &[], None);
    let models = team_member_granted_models(&token, Some(&team), None, true, &world)
        .await
        .unwrap();
    assert_eq!(models, Vec::<String>::new());
    assert!(world.effects().is_empty());

    let preloaded = membership_row("u1", "team-1", Some(&["preloaded"]));
    let preloaded_models =
        team_member_granted_models(&token, Some(&team), Some(preloaded), true, &world)
            .await
            .unwrap();
    assert_eq!(preloaded_models, owned(&["preloaded"]));
    assert!(world.effects().is_empty());
}
