use std::time::Duration;

use betterlite_db::identity_rows::user::LiteLLMUserTable;
use rstest::rstest;
use tokio::time::timeout;

use super::support::{Effect, Logs, Script, World, membership, query, team, unavailable, user};
use crate::auth_identity::errors::AuthError;
use crate::auth_team_lookup::error::{TeamFailure, TeamLookupError};
use crate::auth_team_lookup::lookup::TeamSource;
use crate::auth_user_lookup::error::{UserLookupError, UserReadFailure};
use crate::identity_grants::records::{
    DegradedRead, GrantDenial, GrantOutcome, IdentityLoadError, ResolvedGrants, UserQuery,
};
use crate::membership_load::error::MembershipLoadError;

fn user_effect(user_id: &str) -> Effect {
    Effect::User {
        user_id: Some(user_id.to_owned()),
        sso_user_id: None,
        user_email: None,
        upsert: false,
        check_db_only: None,
    }
}

fn membership_effect(user_id: &str, team_id: &str) -> Effect {
    Effect::Membership {
        user_id: user_id.to_owned(),
        team_id: team_id.to_owned(),
    }
}

fn team_effect(team_id: &str) -> Effect {
    Effect::Team {
        team_id: team_id.to_owned(),
        source: TeamSource::CacheThenDatabase,
        upsert: false,
    }
}

fn resolved(outcome: GrantOutcome) -> ResolvedGrants {
    match outcome {
        GrantOutcome::Resolved(grants) => *grants,
        GrantOutcome::Denied(denial) => panic!("the grants resolve, not {denial:?}"),
        GrantOutcome::Degraded(read) => panic!("the grants resolve, not degraded: {read}"),
    }
}

fn denied(outcome: GrantOutcome) -> GrantDenial {
    match outcome {
        GrantOutcome::Denied(denial) => denial,
        GrantOutcome::Resolved(grants) => panic!("the grants are denied, not {grants:?}"),
        GrantOutcome::Degraded(read) => panic!("the grants are denied, not degraded: {read}"),
    }
}

fn degraded(outcome: GrantOutcome) -> DegradedRead {
    match outcome {
        GrantOutcome::Degraded(read) => read,
        GrantOutcome::Resolved(grants) => panic!("the read degrades, not {grants:?}"),
        GrantOutcome::Denied(denial) => panic!("the read degrades, not {denial:?}"),
    }
}

#[tokio::test]
async fn user_hints_and_upsert_forwarded() {
    let world = World::new().with_user(Ok(Some(user("user-1", &[]))));
    let hinted = UserQuery {
        user_id: Some("user-1".to_owned()),
        user_email: Some("user@example.com".to_owned()),
        sso_user_id: Some("sso-1".to_owned()),
        upsert: true,
    };
    let logs = Logs::default();
    let guard = logs.install();
    let grants = resolved(world.resolver().resolve(&hinted, None).await);
    drop(guard);
    let expected = Effect::User {
        user_id: Some("user-1".to_owned()),
        sso_user_id: Some("sso-1".to_owned()),
        user_email: Some("user@example.com".to_owned()),
        upsert: true,
        check_db_only: None,
    };
    assert_eq!(world.effects(), vec![expected]);
    assert_eq!(grants.identity.effective_user_id.as_deref(), Some("user-1"));
    assert_eq!(grants.identity.user.unwrap().user_id, "user-1");
    assert!(grants.identity.membership.is_none());
    assert!(
        logs.lines().is_empty(),
        "no rebind is logged: {:?}",
        logs.lines()
    );
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
#[tokio::test]
async fn falsey_user_skips_lookup(#[case] user_id: Option<&str>) {
    let world = World::new().with_team(Ok(team("team-1")));
    let identity = world
        .resolver()
        .resolve_identity(&query(user_id), Some("team-1"))
        .await
        .unwrap();
    assert!(identity.user.is_none());
    assert_eq!(identity.effective_user_id.as_deref(), user_id);
    assert!(world.effects().is_empty(), "{:?}", world.effects());
    let grants = resolved(
        world
            .resolver()
            .resolve(&query(user_id), Some("team-1"))
            .await,
    );
    assert_eq!(world.effects(), vec![team_effect("team-1")]);
    assert!(grants.team.is_some());
}

#[tokio::test]
async fn canonical_rebind_before_membership() {
    let world = World::new()
        .with_user(Ok(Some(user("legacy-id", &["team-1"]))))
        .with_membership(Ok(Some(membership("legacy-id", "team-1"))))
        .with_team(Ok(team("team-1")));
    let logs = Logs::default();
    let guard = logs.install();
    let grants = resolved(
        world
            .resolver()
            .resolve(&query(Some("new-id")), Some("team-1"))
            .await,
    );
    drop(guard);
    let effects = vec![
        user_effect("new-id"),
        membership_effect("legacy-id", "team-1"),
        team_effect("team-1"),
    ];
    assert_eq!(world.effects(), effects);
    assert_eq!(
        grants.identity.effective_user_id.as_deref(),
        Some("legacy-id")
    );
    assert_eq!(grants.identity.membership.unwrap().user_id, "legacy-id");
    let lines = logs.lines();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].starts_with("DEBUG "), "{lines:?}");
    for part in ["rebinding user_id", "\"new-id\"", "\"legacy-id\""] {
        assert!(lines[0].contains(part), "{part} in {lines:?}");
    }
}

#[tokio::test]
async fn empty_row_id_keeps_requested_id() {
    let world = World::new()
        .with_user(Ok(Some(user("", &[]))))
        .with_membership(Ok(None));
    let identity = world
        .resolver()
        .resolve_identity(&query(Some("user-1")), Some("team-1"))
        .await
        .unwrap();
    assert_eq!(identity.effective_user_id.as_deref(), Some("user-1"));
    let effects = vec![user_effect("user-1"), membership_effect("user-1", "team-1")];
    assert_eq!(world.effects(), effects);
}

#[tokio::test]
async fn no_user_still_loads_membership() {
    let world = World::new()
        .with_user(Ok(None))
        .with_membership(Ok(Some(membership("user-1", "team-1"))))
        .with_team(Ok(team("team-1")));
    let grants = resolved(
        world
            .resolver()
            .resolve(&query(Some("user-1")), Some("team-1"))
            .await,
    );
    let effects = vec![
        user_effect("user-1"),
        membership_effect("user-1", "team-1"),
        team_effect("team-1"),
    ];
    assert_eq!(world.effects(), effects);
    assert!(grants.identity.user.is_none());
    assert_eq!(grants.identity.effective_user_id.as_deref(), Some("user-1"));
    assert_eq!(grants.identity.membership.unwrap().team_id, "team-1");
}

#[rstest]
#[case::not_found(UserLookupError::NotFound(AuthError::UserNotFound { user_id: "user-1".to_owned() }), true)]
#[case::no_database(UserLookupError::NoDatabase, false)]
#[case::unavailable(UserLookupError::Unavailable(unavailable()), false)]
#[case::missing(UserLookupError::UserMissing(UserReadFailure { user_id: "user-1".to_owned(), cause: unavailable() }), false)]
#[case::pre_lookup(UserLookupError::PreLookup(unavailable()), false)]
#[tokio::test]
async fn user_error_classification(#[case] failure: UserLookupError, #[case] gone: bool) {
    let message = failure.to_string();
    let world = World::new().with_user(Err(failure));
    let outcome = world
        .resolver()
        .resolve(&query(Some("user-1")), Some("team-1"))
        .await;
    assert_eq!(world.effects(), vec![user_effect("user-1")]);
    if gone {
        let user_id = "user-1".to_owned();
        assert_eq!(denied(outcome), GrantDenial::UserGone { user_id });
        return;
    }
    match degraded(outcome) {
        DegradedRead::Identity(IdentityLoadError::User(kept)) => {
            assert_eq!(kept.to_string(), message);
        }
        other @ (DegradedRead::Identity(IdentityLoadError::Membership(_))
        | DegradedRead::Team(_)) => {
            panic!("the user read degrades: {other:?}")
        }
    }
}

#[tokio::test]
async fn identity_errors_surface_unclassified() {
    let not_found = AuthError::UserNotFound {
        user_id: "user-1".to_owned(),
    };
    let world = World::new().with_user(Err(UserLookupError::NotFound(not_found)));
    let surfaced = world
        .resolver()
        .resolve_identity(&query(Some("user-1")), Some("team-1"))
        .await
        .unwrap_err();
    assert!(surfaced.proves_user_gone(), "{surfaced}");
}

#[rstest]
#[case::no_database(MembershipLoadError::NoDatabase)]
#[case::load_lost(MembershipLoadError::LoadLost)]
#[tokio::test]
async fn membership_error_classification(#[case] failure: MembershipLoadError) {
    let message = failure.to_string();
    let world = World::new()
        .with_user(Ok(Some(user("user-1", &["team-1"]))))
        .with_membership(Err(failure));
    let outcome = world
        .resolver()
        .resolve(&query(Some("user-1")), Some("team-1"))
        .await;
    let effects = vec![user_effect("user-1"), membership_effect("user-1", "team-1")];
    assert_eq!(world.effects(), effects);
    match degraded(outcome) {
        DegradedRead::Identity(error @ IdentityLoadError::Membership(_)) => {
            assert!(!error.proves_user_gone());
            assert_eq!(error.to_string(), message);
        }
        other @ (DegradedRead::Identity(IdentityLoadError::User(_)) | DegradedRead::Team(_)) => {
            panic!("the membership read degrades: {other:?}")
        }
    }
}

#[tokio::test]
async fn team_none_short_circuit() {
    let world = World::new().with_user(Ok(Some(user("user-1", &[]))));
    let grants = resolved(world.resolver().resolve(&query(Some("user-1")), None).await);
    assert_eq!(world.effects(), vec![user_effect("user-1")]);
    assert!(grants.team.is_none());
    assert!(grants.identity.membership.is_none());
}

#[rstest]
#[case::other_team(&["team-2"])]
#[case::case_differs(&["Team-1"])]
#[case::no_teams(&[])]
#[tokio::test]
async fn teams_array_controls_membership(#[case] teams: &[&str]) {
    let world = World::new()
        .with_user(Ok(Some(user("legacy-id", teams))))
        .with_membership(Ok(Some(membership("legacy-id", "team-1"))));
    let outcome = world
        .resolver()
        .resolve(&query(Some("user-1")), Some("team-1"))
        .await;
    let expected = GrantDenial::NotAMember {
        user_id: "legacy-id".to_owned(),
        team_id: "team-1".to_owned(),
    };
    assert_eq!(denied(outcome), expected);
    let effects = vec![
        user_effect("user-1"),
        membership_effect("legacy-id", "team-1"),
    ];
    assert_eq!(world.effects(), effects);
}

#[tokio::test]
async fn missing_membership_not_denial() {
    let world = World::new()
        .with_user(Ok(Some(user("user-1", &["team-1"]))))
        .with_membership(Ok(None))
        .with_team(Ok(team("team-1")));
    let grants = resolved(
        world
            .resolver()
            .resolve(&query(Some("user-1")), Some("team-1"))
            .await,
    );
    assert!(grants.identity.membership.is_none());
    assert_eq!(grants.team.unwrap().team.fields.base.team_id, "team-1");
    assert_eq!(world.effects().len(), 3);
}

#[rstest]
#[case::no_user(Ok(None))]
#[case::listed(Ok(Some(user("user-1", &[""]))))]
#[tokio::test]
async fn empty_team_still_loaded(
    #[case] answer: Result<Option<LiteLLMUserTable>, UserLookupError>,
) {
    let world = World::new().with_user(answer).with_team(Ok(team("")));
    let grants = resolved(
        world
            .resolver()
            .resolve(&query(Some("user-1")), Some(""))
            .await,
    );
    assert_eq!(
        world.effects(),
        vec![user_effect("user-1"), team_effect("")]
    );
    assert!(grants.identity.membership.is_none());
    assert!(grants.team.is_some());
}

#[tokio::test]
async fn empty_team_unlisted_is_not_a_member() {
    let world = World::new().with_user(Ok(Some(user("user-1", &["team-1"]))));
    let outcome = world
        .resolver()
        .resolve(&query(Some("user-1")), Some(""))
        .await;
    let expected = GrantDenial::NotAMember {
        user_id: "user-1".to_owned(),
        team_id: String::new(),
    };
    assert_eq!(denied(outcome), expected);
    assert_eq!(world.effects(), vec![user_effect("user-1")]);
}

#[tokio::test]
async fn missing_user_allows_team_load() {
    let world = World::new()
        .with_user(Ok(None))
        .with_membership(Ok(None))
        .with_team(Ok(team("team-1")));
    let grants = resolved(
        world
            .resolver()
            .resolve(&query(Some("user-1")), Some("team-1"))
            .await,
    );
    assert!(grants.identity.user.is_none());
    assert_eq!(grants.team.unwrap().team.fields.base.team_id, "team-1");
    let effects = vec![
        user_effect("user-1"),
        membership_effect("user-1", "team-1"),
        team_effect("team-1"),
    ];
    assert_eq!(world.effects(), effects);
}

fn throttled(team_id: &str) -> TeamLookupError {
    TeamLookupError::TeamUnavailable {
        team_id: team_id.to_owned(),
        cause: Box::new(TeamFailure::Throttled),
    }
}

#[rstest]
#[case::not_found(TeamLookupError::TeamNotFound { team_id: "elsewhere".to_owned() }, true)]
#[case::unavailable(throttled("team-1"), false)]
#[case::no_database(TeamLookupError::NoDatabase, false)]
#[case::not_in_cache(TeamLookupError::NotInCache { team_id: "team-1".to_owned() }, false)]
#[case::alias_missing(TeamLookupError::AliasNotFound { team_alias: "team-1".to_owned() }, false)]
#[case::group_missing(TeamLookupError::AccessGroupNotFound { access_group_id: "g1".to_owned() }, false)]
#[tokio::test]
async fn team_error_classification(#[case] failure: TeamLookupError, #[case] gone: bool) {
    let message = failure.to_string();
    let world = World::new()
        .with_user(Ok(None))
        .with_membership(Ok(None))
        .with_team(Err(failure));
    let outcome = world
        .resolver()
        .resolve(&query(Some("user-1")), Some("team-1"))
        .await;
    assert_eq!(world.effects().len(), 3);
    if gone {
        let team_id = "team-1".to_owned();
        assert_eq!(denied(outcome), GrantDenial::TeamGone { team_id });
        return;
    }
    match degraded(outcome) {
        DegradedRead::Team(kept) => assert_eq!(kept.to_string(), message),
        other @ DegradedRead::Identity(_) => panic!("the team read degrades: {other:?}"),
    }
}

#[tokio::test(start_paused = true)]
async fn cancellation_not_degraded() {
    let caller = query(Some("user-1"));
    let stalled_user = World::new();
    *stalled_user.user.lock().unwrap() = Script::Pending;
    let user_resolver = stalled_user.resolver();
    let pending_user = user_resolver.resolve(&caller, Some("team-1"));
    let user_elapsed = timeout(Duration::from_secs(5), pending_user)
        .await
        .unwrap_err();
    assert_eq!(user_elapsed.to_string(), "deadline has elapsed");
    assert_eq!(stalled_user.effects(), vec![user_effect("user-1")]);

    let stalled_team = World::new().with_user(Ok(None)).with_membership(Ok(None));
    *stalled_team.team.lock().unwrap() = Script::Pending;
    let team_resolver = stalled_team.resolver();
    let pending_team = team_resolver.resolve(&caller, Some("team-1"));
    let team_elapsed = timeout(Duration::from_secs(5), pending_team)
        .await
        .unwrap_err();
    assert_eq!(team_elapsed.to_string(), "deadline has elapsed");
    assert_eq!(stalled_team.effects().len(), 3);
}
