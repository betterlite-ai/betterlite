use rstest::rstest;

use super::{FakeReads, Scripted, grants, names};

#[rstest]
#[case::cached(false)]
#[case::fresh(true)]
#[tokio::test]
async fn user_teams_are_distinct_in_first_seen_order(#[case] fresh: bool) {
    let server_grants = grants(FakeReads {
        user_teams: Scripted::Value(Some(names(&[
            "team-b",
            "",
            "team-a",
            "litellm-dashboard",
            "team-b",
            "team-c",
        ]))),
        ..FakeReads::standard()
    });
    assert_eq!(
        server_grants.user_team_ids("user-1", fresh).await,
        names(&["team-b", "team-a", "team-c"])
    );
    assert_eq!(
        server_grants.reads.calls(),
        Vec::from([format!("user_team_ids:user-1:{fresh}")])
    );
}

#[rstest]
#[case::missing_user(Scripted::Value(None))]
#[case::no_teams(Scripted::Value(Some(Vec::new())))]
#[case::read_fault(Scripted::Fault)]
#[tokio::test]
async fn user_teams_are_empty_when_nothing_resolves(
    #[case] user_teams: Scripted<Option<Vec<String>>>,
) {
    let server_grants = grants(FakeReads {
        user_teams,
        ..FakeReads::standard()
    });
    assert!(
        server_grants
            .user_team_ids("user-1", false)
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn user_teams_without_a_database_read_nothing() {
    let server_grants = grants(FakeReads {
        database: false,
        user_teams: Scripted::Value(Some(names(&["team-a"]))),
        ..FakeReads::standard()
    });
    assert!(
        server_grants
            .user_team_ids("user-1", false)
            .await
            .is_empty()
    );
    assert!(server_grants.reads.calls().is_empty());
}
