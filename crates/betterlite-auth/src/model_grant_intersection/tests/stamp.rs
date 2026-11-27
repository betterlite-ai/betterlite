use crate::model_grant_intersection::stamp::{
    StampRequest, matched_model_access_groups, stamp_matched_groups,
};
use crate::model_grant_intersection::tests::support::{
    Logs, World, auth_object, group_set, list, owned, text,
};
use crate::resolved_key::auth::UserAPIKeyAuth;

async fn stamp_from_registry(
    auth: &mut UserAPIKeyAuth,
    models: &[String],
    registry: &[&str],
    expected: &str,
) {
    let world = World::new()
        .with_registry(Ok(Some(group_set(registry))))
        .with_serving(&[("gpt-4o", &["tier-a", "tier-b"])]);
    let matched = stamp_matched_groups(StampRequest {
        models: Some(models),
        auth: Some(&mut *auth),
        team: None,
        project: None,
        membership: None,
        membership_loaded: false,
        groups: Some(&world),
        sources: Some(&world),
    })
    .await;
    assert_eq!(matched, owned(&[expected]));
    assert_eq!(
        matched_model_access_groups(auth),
        owned(&[expected]).as_slice()
    );
}

#[tokio::test]
async fn matched_group_stamping() {
    let mut auth = auth_object(&[
        ("models", list(&["tier-a", "tier-b"])),
        ("team_id", text("team-1")),
    ]);
    let models = owned(&["gpt-4o"]);
    stamp_from_registry(&mut auth, &models, &["tier-a"], "tier-a").await;
    stamp_from_registry(&mut auth, &models, &["tier-b"], "tier-b").await;
}

#[tokio::test]
async fn stamp_exception_debug_drop() {
    let logs = Logs::default();
    let _guard = logs.install();
    let mut auth = auth_object(&[("models", list(&["tier-a"]))]);
    auth.extra.matched_model_access_groups = Some(owned(&["previous"]));
    let models = owned(&["gpt-4o"]);
    let world = World::new().with_registry(Ok(None)).with_broken_router();
    let matched = stamp_matched_groups(StampRequest {
        models: Some(&models),
        auth: Some(&mut auth),
        team: None,
        project: None,
        membership: None,
        membership_loaded: false,
        groups: Some(&world),
        sources: Some(&world),
    })
    .await;
    assert!(matched.is_empty());
    assert_eq!(
        matched_model_access_groups(&auth),
        owned(&["previous"]).as_slice()
    );
    assert!(
        logs.lines()
            .iter()
            .any(|line| line.starts_with("DEBUG") && line.contains("attribution failed"))
    );
}

async fn previous_stamp_survives_empty_match() {
    let models = owned(&["gpt-4o"]);
    let mut auth = auth_object(&[("models", list(&["gpt-4o"]))]);
    auth.extra.matched_model_access_groups = Some(owned(&["previous"]));
    let world = World::new()
        .with_registry(Ok(Some(group_set(&["tier-a"]))))
        .with_serving(&[("gpt-4o", &["tier-a"])]);
    let matched = stamp_matched_groups(StampRequest {
        models: Some(&models),
        auth: Some(&mut auth),
        team: None,
        project: None,
        membership: None,
        membership_loaded: false,
        groups: Some(&world),
        sources: Some(&world),
    })
    .await;
    assert!(matched.is_empty());
    assert_eq!(
        matched_model_access_groups(&auth),
        owned(&["previous"]).as_slice()
    );
}

async fn fresh_auth_stays_unstamped() {
    let models = owned(&["gpt-4o"]);
    let world = World::new()
        .with_registry(Ok(Some(group_set(&["tier-a"]))))
        .with_serving(&[("gpt-4o", &["tier-a"])]);
    let mut fresh = auth_object(&[("models", list(&["gpt-4o"]))]);
    let matched = stamp_matched_groups(StampRequest {
        models: Some(&models),
        auth: Some(&mut fresh),
        team: None,
        project: None,
        membership: None,
        membership_loaded: false,
        groups: Some(&world),
        sources: Some(&world),
    })
    .await;
    assert!(matched.is_empty());
    assert!(matched_model_access_groups(&fresh).is_empty());
}

async fn missing_auth_writes_nothing() {
    let models = owned(&["gpt-4o"]);
    let world = World::new();
    let matched = stamp_matched_groups(StampRequest {
        models: Some(&models),
        auth: None,
        team: None,
        project: None,
        membership: None,
        membership_loaded: false,
        groups: Some(&world),
        sources: Some(&world),
    })
    .await;
    assert!(matched.is_empty());
    assert!(world.effects().is_empty());
}

#[tokio::test]
async fn empty_stamp_keeps_previous() {
    previous_stamp_survives_empty_match().await;
    fresh_auth_stays_unstamped().await;
    missing_auth_writes_nothing().await;
}
