use rstest::rstest;

use crate::model_grant_intersection::member::team_member_granted_models;
use crate::model_grant_intersection::tests::support::{
    World, grant_token, membership_row, owned, team_row, text,
};

#[derive(Clone, Copy)]
enum MembershipAnswer {
    Missing,
    Empty,
    Granted,
}

#[rstest]
#[case::missing_row(MembershipAnswer::Missing, &[])]
#[case::empty_allowed_models(MembershipAnswer::Empty, &[])]
#[case::allowed_models(MembershipAnswer::Granted, &["member-group"])]
#[tokio::test]
async fn member_scope_uses_only_present_allowed_models(
    #[case] membership_answer: MembershipAnswer,
    #[case] expected: &[&str],
) {
    let team = team_row("team-1", &[], None);
    let token = grant_token(&[("user_id", text("u1"))]);
    let world = match membership_answer {
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
    let models = team_member_granted_models(&token, Some(&team), None, false, &world)
        .await
        .unwrap();
    assert_eq!(models, owned(expected));
    assert_eq!(
        world.effects().len(),
        1,
        "a present member reads the membership once"
    );
}

#[rstest]
#[case::no_team(None, Some("u1"))]
#[case::no_user(Some("team-1"), None)]
#[tokio::test]
async fn missing_member_inputs_empty(#[case] team_id: Option<&str>, #[case] user_id: Option<&str>) {
    let team = team_id.map(|id| team_row(id, &[], None));
    let token = user_id.map_or_else(
        || grant_token(&[]),
        |id| grant_token(&[("user_id", text(id))]),
    );
    let world = World::new();
    let models = team_member_granted_models(&token, team.as_ref(), None, false, &world)
        .await
        .unwrap();
    assert_eq!(models, owned(&[]));
    assert!(
        world.effects().is_empty(),
        "a missing team or user reads no membership"
    );
}
