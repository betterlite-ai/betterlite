use rstest::rstest;

use crate::model_grant_intersection::member::team_member_granted_models;
use crate::model_grant_intersection::tests::support::{
    World, grant_token, membership_row, owned, team_row, text,
};

#[derive(Clone, Copy)]
enum MembershipAnswer {
    NotRead,
    Missing,
    Empty,
    Granted,
}

#[rstest]
#[case::no_team(None, Some("u1"), MembershipAnswer::NotRead, &[], false)]
#[case::no_user(Some("team-1"), None, MembershipAnswer::NotRead, &[], false)]
#[case::missing_row(Some("team-1"), Some("u1"), MembershipAnswer::Missing, &[], true)]
#[case::empty_allowed_models(Some("team-1"), Some("u1"), MembershipAnswer::Empty, &[], true)]
#[case::allowed_models(Some("team-1"), Some("u1"), MembershipAnswer::Granted, &["member-group"], true)]
#[tokio::test]
async fn member_scope_uses_only_present_allowed_models(
    #[case] team_id: Option<&str>,
    #[case] user_id: Option<&str>,
    #[case] membership_answer: MembershipAnswer,
    #[case] expected: &[&str],
    #[case] queries_membership: bool,
) {
    let team = team_id.map(|id| team_row(id, &[], None));
    let token = user_id.map_or_else(
        || grant_token(&[]),
        |id| grant_token(&[("user_id", text(id))]),
    );
    let world = match membership_answer {
        MembershipAnswer::NotRead => World::new(),
        MembershipAnswer::Missing => World::new().with_membership(Ok(None)),
        MembershipAnswer::Empty => {
            World::new().with_membership(Ok(Some(membership_row("u1", "team-1", None))))
        }
        MembershipAnswer::Granted => World::new().with_membership(Ok(Some(membership_row(
            "u1",
            "team-1",
            Some(&["member-group"]),
        )))),
    };
    let models = team_member_granted_models(&token, team.as_ref(), None, false, &world)
        .await
        .unwrap();
    assert_eq!(models, owned(expected));
    assert_eq!(
        world.effects().len(),
        usize::from(queries_membership),
        "membership is read only when both team and user are present"
    );
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
