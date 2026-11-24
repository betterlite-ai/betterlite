use std::sync::Arc;

use betterlite_auth::user_roles::UserRole;

use super::super::authz::{AuthorizationRequest, ScopeClause, authorize};
use super::super::error::LogListError;
use super::support::{Answers, Events, FakeScope, kinds, seen};
use crate::spend::log_query::decode_spend_log_query;
use crate::spend::log_query::prepare::SpendLogRoute;
use crate::team_access::caller::TeamCaller;

fn caller(user_id: Option<&str>, user_role: Option<UserRole>) -> TeamCaller {
    TeamCaller {
        user_id: user_id.map(str::to_owned),
        user_role,
        team_id: None,
        team_service_account: false,
    }
}

fn scope(events: &Events) -> FakeScope {
    FakeScope {
        events: Arc::clone(events),
        permitted: Vec::new(),
        teams: Vec::new(),
        answers: Answers::default(),
    }
}

async fn scope_decision(
    raw_query: Option<&str>,
    route: SpendLogRoute,
    caller: &TeamCaller,
    scope: &FakeScope,
) -> Result<ScopeClause, LogListError> {
    let query = decode_spend_log_query(raw_query);
    authorize(&AuthorizationRequest {
        caller,
        query: &query,
        route,
        scope: Some(scope),
    })
    .await
    .map(|authorization| authorization.scope)
}

#[tokio::test]
async fn an_admin_is_scoped_nowhere_and_asks_no_port() {
    let events = Events::default();
    let mut fake = scope(&events);
    fake.permitted = vec![String::from("team-1")];
    fake.answers.precheck = false;
    let admin = caller(Some("root"), Some(UserRole::ProxyAdmin));
    let found = scope_decision(
        Some("request_id=r1&team_id=t9"),
        SpendLogRoute::Ui,
        &admin,
        &fake,
    )
    .await;
    assert_eq!(found.unwrap(), ScopeClause::None);
    assert!(seen(&events).is_empty());
}

#[tokio::test]
async fn an_internal_user_is_scoped_to_own_rows() {
    let events = Events::default();
    let alice = caller(Some("alice"), Some(UserRole::InternalUser));
    let found = scope_decision(None, SpendLogRoute::Ui, &alice, &scope(&events)).await;
    assert_eq!(
        found.unwrap(),
        ScopeClause::OwnUser {
            user_id: Some(String::from("alice")),
        }
    );
    assert_eq!(seen(&events), ["permitted"]);
}

#[tokio::test]
async fn permitted_teams_expand_the_scope_to_an_or_clause() {
    let events = Events::default();
    let mut fake = scope(&events);
    fake.permitted = vec![String::from("team-1"), String::from("team-2")];
    let alice = caller(Some("alice"), Some(UserRole::InternalUserViewer));
    let found = scope_decision(None, SpendLogRoute::Ui, &alice, &fake).await;
    assert_eq!(
        found.unwrap(),
        ScopeClause::OwnUserOrTeams {
            user_id: Some(String::from("alice")),
            team_ids: vec![String::from("team-1"), String::from("team-2")],
        }
    );
}

#[rstest::rstest]
#[case::permitted(true)]
#[case::forbidden(false)]
#[tokio::test]
async fn a_team_filter_needs_a_visible_team(#[case] visible: bool) {
    let events = Events::default();
    let mut fake = scope(&events);
    if visible {
        fake.teams = vec![String::from("team-1")];
    }
    let alice = caller(Some("alice"), Some(UserRole::InternalUser));
    let found = scope_decision(Some("team_id=team-1"), SpendLogRoute::Ui, &alice, &fake).await;
    if visible {
        assert_eq!(found.unwrap(), ScopeClause::None);
        assert_eq!(seen(&events), ["team team-1"]);
    } else {
        assert!(matches!(found, Err(LogListError::ForbiddenTeam { .. })));
    }
}

#[tokio::test]
async fn a_ui_id_lookup_is_prechecked_even_with_a_search() {
    let events = Events::default();
    let alice = caller(Some("alice"), Some(UserRole::InternalUser));
    let found = scope_decision(
        Some("request_id=r1&search=r1"),
        SpendLogRoute::Ui,
        &alice,
        &scope(&events),
    )
    .await;
    assert_eq!(
        found.unwrap(),
        ScopeClause::OwnUser {
            user_id: Some(String::from("alice")),
        }
    );
    assert_eq!(kinds(&events), ["precheck", "permitted"]);
}

#[tokio::test]
async fn the_public_route_never_prechecks_an_id() {
    let events = Events::default();
    let alice = caller(Some("alice"), Some(UserRole::InternalUser));
    let found = scope_decision(
        Some("request_id=r1&start_date=2025-01-02&end_date=2025-01-03"),
        SpendLogRoute::PublicV2,
        &alice,
        &scope(&events),
    )
    .await;
    assert!(matches!(found.unwrap(), ScopeClause::OwnUser { .. }));
    assert_eq!(seen(&events), ["permitted"]);
}

#[tokio::test]
async fn a_role_without_log_view_gets_no_user_scope_except_for_an_id_lookup() {
    let events = Events::default();
    let customer = caller(Some("c1"), Some(UserRole::Customer));
    let listed = scope_decision(None, SpendLogRoute::Ui, &customer, &scope(&events)).await;
    assert_eq!(listed.unwrap(), ScopeClause::None);
    assert!(seen(&events).is_empty());
    let looked_up = scope_decision(
        Some("request_id=r1"),
        SpendLogRoute::Ui,
        &customer,
        &scope(&events),
    )
    .await;
    assert_eq!(
        looked_up.unwrap(),
        ScopeClause::OwnUser {
            user_id: Some(String::from("c1")),
        }
    );
}

#[tokio::test]
async fn a_null_caller_id_stays_a_predicate() {
    let events = Events::default();
    let nameless = caller(None, Some(UserRole::InternalUser));
    let found = scope_decision(
        Some("request_id=r1"),
        SpendLogRoute::Ui,
        &nameless,
        &scope(&events),
    )
    .await;
    assert_eq!(found.unwrap(), ScopeClause::OwnUser { user_id: None });
}

#[tokio::test]
async fn a_missing_scope_port_fails_an_id_lookup_closed() {
    let query = decode_spend_log_query(Some("request_id=r1"));
    let alice = caller(Some("alice"), Some(UserRole::InternalUser));
    let refused = authorize(&AuthorizationRequest {
        caller: &alice,
        query: &query,
        route: SpendLogRoute::Ui,
        scope: None,
    })
    .await;
    assert!(matches!(
        refused,
        Err(LogListError::ForbiddenRequestId { .. })
    ));
}
