use crate::auth_org_lookup::error::OrgLookupError;
use crate::model_grant_intersection::grant::{AttributionRequest, granted_model_lists};
use crate::model_grant_intersection::tests::support::{
    Effect, Logs, World, grant_token, list, membership_row, organization_row, owned, project_row,
    team_row, text,
};

async fn five_scope_lists() -> (World, Vec<Vec<String>>) {
    let world = World::new()
        .with_membership(Ok(Some(membership_row(
            "u1",
            "team-1",
            Some(&["member-group"]),
        ))))
        .with_org(Ok(Some(organization_row("org-1", &["org-group"]))));
    let token = grant_token(&[
        ("models", list(&["all-team-models"])),
        ("team_models", list(&["team-group"])),
        ("team_id", text("team-1")),
        ("user_id", text("u1")),
        ("org_id", text("org-1")),
    ]);
    let team = team_row("team-1", &["team-group"], None);
    let project = project_row("p1", &["project-group"]);
    let lists = granted_model_lists(AttributionRequest {
        token: &token,
        team: Some(&team),
        project: Some(&project),
        membership: None,
        membership_loaded: false,
        sources: &world,
    })
    .await
    .unwrap();
    (world, lists)
}

#[tokio::test]
async fn attribution_five_list_order() {
    let (world, lists) = five_scope_lists().await;
    assert_eq!(
        lists,
        vec![
            owned(&["team-group"]),
            owned(&["team-group"]),
            owned(&["member-group"]),
            owned(&["project-group"]),
            owned(&["org-group"]),
        ]
    );
    assert_eq!(
        world.effects(),
        vec![
            Effect::Membership {
                user_id: "u1".to_owned(),
                team_id: "team-1".to_owned(),
            },
            Effect::Org("org-1".to_owned()),
        ]
    );
}

#[tokio::test]
async fn key_org_before_team_org() {
    let key_world = World::new().with_org(Ok(Some(organization_row("key-org", &["key-group"]))));
    let key_token = grant_token(&[
        ("models", list(&["*"])),
        ("user_id", text("u1")),
        ("org_id", text("key-org")),
    ]);
    let key_team = team_row("team-1", &[], Some("team-org"));
    let lists = granted_model_lists(AttributionRequest {
        token: &key_token,
        team: Some(&key_team),
        project: None,
        membership: None,
        membership_loaded: true,
        sources: &key_world,
    })
    .await
    .unwrap();
    assert_eq!(lists[4], owned(&["key-group"]));
    assert_eq!(key_world.effects(), vec![Effect::Org("key-org".to_owned())]);
}

#[tokio::test]
async fn team_org_fallback() {
    let world = World::new().with_org(Ok(Some(organization_row("team-org", &["team-group"]))));
    let token = grant_token(&[("models", list(&["*"]))]);
    let team = team_row("team-1", &[], Some("team-org"));
    let lists = granted_model_lists(AttributionRequest {
        token: &token,
        team: Some(&team),
        project: None,
        membership: None,
        membership_loaded: true,
        sources: &world,
    })
    .await
    .unwrap();
    assert_eq!(lists[4], owned(&["team-group"]));
    assert_eq!(world.effects(), vec![Effect::Org("team-org".to_owned())]);
}

#[tokio::test]
async fn empty_key_org_is_explicit() {
    let world = World::new().with_org(Ok(None));
    let token = grant_token(&[("models", list(&["*"])), ("org_id", text(""))]);
    let team = team_row("team-1", &[], Some("team-org"));
    let lists = granted_model_lists(AttributionRequest {
        token: &token,
        team: Some(&team),
        project: None,
        membership: None,
        membership_loaded: true,
        sources: &world,
    })
    .await
    .unwrap();
    assert_eq!(lists[4], Vec::<String>::new());
    assert_eq!(world.effects(), vec![Effect::Org(String::new())]);
}

#[tokio::test]
async fn missing_org_skips_lookup() {
    let world = World::new();
    let team = team_row("team-1", &[], None);
    let token = grant_token(&[("models", list(&["*"]))]);
    let lists = granted_model_lists(AttributionRequest {
        token: &token,
        team: Some(&team),
        project: None,
        membership: None,
        membership_loaded: true,
        sources: &world,
    })
    .await
    .unwrap();
    assert_eq!(lists[4], Vec::<String>::new());
    assert!(world.effects().is_empty());
}

#[tokio::test]
async fn org_error_debug_empty() {
    let logs = Logs::default();
    let _guard = logs.install();
    let world = World::new().with_org(Err(OrgLookupError::NoDatabase));
    let token = grant_token(&[("models", list(&["*"])), ("org_id", text("org-1"))]);
    let lists = granted_model_lists(AttributionRequest {
        token: &token,
        team: None,
        project: None,
        membership: None,
        membership_loaded: true,
        sources: &world,
    })
    .await
    .unwrap();
    assert_eq!(lists[4], Vec::<String>::new());
    assert!(
        logs.lines()
            .iter()
            .any(|line| line.starts_with("DEBUG") && line.contains("org lookup failed"))
    );
}
