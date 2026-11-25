use std::collections::BTreeMap;

use rstest::rstest;

use super::super::error::ServerGrantsError;
use super::{
    FakeReads, Scripted, four_source_row, grant_auth, grants, team_record, toolset_grants,
};

#[derive(Clone, Copy, Debug)]
enum Fault {
    Team,
    MissingTeam,
    TeamAccessGroups,
    LegacyAccessGroups,
    Toolsets,
}

fn faulted(fault: Fault) -> FakeReads {
    let mut reads = FakeReads {
        team: Scripted::Value(Some(team_record(Some(four_source_row())))),
        unified: Scripted::Value(Vec::from([String::from("srv-team-group")])),
        legacy: Scripted::Value(Vec::from([String::from("srv-group")])),
        toolsets: Scripted::Value(toolset_grants()),
        ..FakeReads::standard()
    };
    match fault {
        Fault::Team => reads.team = Scripted::Fault,
        Fault::MissingTeam => reads.team = Scripted::Value(None),
        Fault::TeamAccessGroups => reads.unified = Scripted::Fault,
        Fault::LegacyAccessGroups => reads.legacy = Scripted::Fault,
        Fault::Toolsets => reads.toolsets = Scripted::Fault,
    }
    reads
}

#[rstest]
#[case::cached(false)]
#[case::fresh(true)]
#[tokio::test]
async fn team_error_unloadable_toolsets_are_returned(#[case] fresh: bool) {
    let server_grants = grants(FakeReads {
        team: Scripted::Value(Some(team_record(Some(four_source_row())))),
        toolsets: Scripted::Value(BTreeMap::new()),
        ..FakeReads::standard()
    });
    let auth = grant_auth(Some("team-1"), fresh);
    let error = server_grants.team_servers(Some(&auth)).await.unwrap_err();
    assert!(
        matches!(error, ServerGrantsError::Unloadable(_)),
        "{error:?}"
    );
}

#[rstest]
#[case::team(Fault::Team)]
#[case::missing_team(Fault::MissingTeam)]
#[case::team_access_groups(Fault::TeamAccessGroups)]
#[case::legacy_access_groups(Fault::LegacyAccessGroups)]
#[case::toolsets(Fault::Toolsets)]
#[tokio::test]
async fn team_error_faults_under_the_fresh_policy_are_returned(#[case] fault: Fault) {
    let server_grants = grants(faulted(fault));
    let auth = grant_auth(Some("team-1"), true);
    let error = server_grants.team_servers(Some(&auth)).await.unwrap_err();
    let expected = match fault {
        Fault::Team | Fault::TeamAccessGroups | Fault::LegacyAccessGroups => {
            matches!(error, ServerGrantsError::Read { .. })
        }
        Fault::MissingTeam => matches!(error, ServerGrantsError::TeamMissing { .. }),
        Fault::Toolsets => matches!(error, ServerGrantsError::Permission { .. }),
    };
    assert!(expected, "{error:?}");
}

#[rstest]
#[case::team(Fault::Team)]
#[case::missing_team(Fault::MissingTeam)]
#[case::team_access_groups(Fault::TeamAccessGroups)]
#[case::legacy_access_groups(Fault::LegacyAccessGroups)]
#[case::toolsets(Fault::Toolsets)]
#[tokio::test]
async fn team_error_other_faults_give_an_empty_list(#[case] fault: Fault) {
    let server_grants = grants(faulted(fault));
    let auth = grant_auth(Some("team-1"), false);
    assert!(
        server_grants
            .team_servers(Some(&auth))
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn team_error_single_team_returns_the_fault_of_a_fresh_read() {
    let server_grants = grants(faulted(Fault::Team));
    let error = server_grants
        .single_team_servers("team-1", true)
        .await
        .unwrap_err();
    assert!(matches!(error, ServerGrantsError::Read { .. }), "{error:?}");
    assert!(
        server_grants
            .single_team_servers("team-1", false)
            .await
            .unwrap()
            .is_empty()
    );
}
