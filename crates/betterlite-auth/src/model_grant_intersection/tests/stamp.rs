use crate::model_grant_intersection::stamp::{
    StampRequest, matched_model_access_groups, stamp_matched_model_access_groups,
};
use crate::model_grant_intersection::tests::support::{
    Logs, World, auth_object, group_set, list, owned, text,
};

#[tokio::test]
async fn matched_group_stamping() {
    let mut auth = auth_object(&[
        ("models", list(&["tier-a", "tier-b"])),
        ("team_id", text("team-1")),
    ]);
    let models = owned(&["gpt-4o"]);
    let first = World::new()
        .with_registry(Ok(Some(group_set(&["tier-a"]))))
        .with_serving(&[("gpt-4o", &["tier-a", "tier-b"])]);
    let matched = stamp_matched_model_access_groups(StampRequest {
        models: Some(&models),
        auth: Some(&mut auth),
        team: None,
        project: None,
        membership: None,
        membership_loaded: false,
        groups: Some(&first),
        sources: Some(&first),
    })
    .await;
    assert_eq!(matched, owned(&["tier-a"]));
    assert_eq!(
        matched_model_access_groups(&auth),
        owned(&["tier-a"]).as_slice()
    );

    let second = World::new()
        .with_registry(Ok(Some(group_set(&["tier-b"]))))
        .with_serving(&[("gpt-4o", &["tier-a", "tier-b"])]);
    let matched = stamp_matched_model_access_groups(StampRequest {
        models: Some(&models),
        auth: Some(&mut auth),
        team: None,
        project: None,
        membership: None,
        membership_loaded: false,
        groups: Some(&second),
        sources: Some(&second),
    })
    .await;
    assert_eq!(matched, owned(&["tier-b"]));
    assert_eq!(
        matched_model_access_groups(&auth),
        owned(&["tier-b"]).as_slice()
    );
}

#[tokio::test]
async fn stamp_exception_debug_drop() {
    let logs = Logs::default();
    let _guard = logs.install();
    let mut auth = auth_object(&[("models", list(&["tier-a"]))]);
    auth.extra.matched_model_access_groups = Some(owned(&["previous"]));
    let models = owned(&["gpt-4o"]);
    let world = World::new().with_registry(Ok(None)).with_broken_router();
    let matched = stamp_matched_model_access_groups(StampRequest {
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

#[tokio::test]
async fn empty_stamp_keeps_previous() {
    let models = owned(&["gpt-4o"]);

    let mut auth = auth_object(&[("models", list(&["gpt-4o"]))]);
    auth.extra.matched_model_access_groups = Some(owned(&["previous"]));
    let world = World::new()
        .with_registry(Ok(Some(group_set(&["tier-a"]))))
        .with_serving(&[("gpt-4o", &["tier-a"])]);
    let matched = stamp_matched_model_access_groups(StampRequest {
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

    let mut fresh = auth_object(&[("models", list(&["gpt-4o"]))]);
    let other = World::new()
        .with_registry(Ok(Some(group_set(&["tier-a"]))))
        .with_serving(&[("gpt-4o", &["tier-a"])]);
    let matched = stamp_matched_model_access_groups(StampRequest {
        models: Some(&models),
        auth: Some(&mut fresh),
        team: None,
        project: None,
        membership: None,
        membership_loaded: false,
        groups: Some(&other),
        sources: Some(&other),
    })
    .await;
    assert!(matched.is_empty());
    assert!(matched_model_access_groups(&fresh).is_empty());

    let untouched = World::new();
    let matched = stamp_matched_model_access_groups(StampRequest {
        models: Some(&models),
        auth: None,
        team: None,
        project: None,
        membership: None,
        membership_loaded: false,
        groups: Some(&untouched),
        sources: Some(&untouched),
    })
    .await;
    assert!(matched.is_empty());
    assert!(untouched.effects().is_empty());
}
