use crate::model_grant_intersection::member::team_member_granted_models;
use crate::model_grant_intersection::tests::support::{
    World, grant_token, membership_row, owned, team_row, text,
};

#[tokio::test]
async fn missing_member_inputs_empty() {
    let cases = [
        (None, None, "unused"),
        (Some("team-1"), None, "unused"),
        (Some("team-1"), Some("u1"), "missing"),
        (Some("team-1"), Some("u1"), "empty"),
    ];
    for (team_id, user_id, membership_case) in cases {
        let team = team_id.map(|id| team_row(id, &[], None));
        let token = user_id.map_or_else(
            || grant_token(&[]),
            |id| grant_token(&[("user_id", text(id))]),
        );
        let world = match membership_case {
            "empty" => World::new().with_membership(Ok(Some(membership_row("u1", "team-1", None)))),
            "missing" => World::new().with_membership(Ok(None)),
            _ => World::new(),
        };
        let models = team_member_granted_models(&token, team.as_ref(), None, false, &world)
            .await
            .unwrap();
        assert_eq!(models, Vec::<String>::new());
        if user_id.is_none() || team_id.is_none() {
            assert!(world.effects().is_empty());
        }
    }
    let world = World::new().with_membership(Ok(Some(membership_row(
        "u1",
        "team-1",
        Some(&["member-group"]),
    ))));
    let team = team_row("team-1", &[], None);
    let token = grant_token(&[("user_id", text("u1"))]);
    let models = team_member_granted_models(&token, Some(&team), None, false, &world)
        .await
        .unwrap();
    assert_eq!(models, owned(&["member-group"]));
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
