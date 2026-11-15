use std::sync::Arc;

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::support::FakeRuntime;
use crate::tracing_routes::access::{TraceAuth, TraceRole};
use crate::tracing_routes::query::{
    QueryAccessError, QueryScope, provide_trace_query_access, provide_trace_query_secret,
    trace_query_scope,
};
use crate::tracing_routes::receiver::TraceRuntime;

fn auth(
    role: TraceRole,
    team_id: Option<&str>,
    token: Option<&str>,
    project_id: Option<&str>,
) -> TraceAuth {
    TraceAuth {
        role,
        team_id: team_id.map(str::to_owned),
        token: token.map(str::to_owned),
        org_id: None,
        project_id: project_id.map(str::to_owned),
    }
}

#[rstest]
#[case::admin(TraceRole::Admin, None, None, None, QueryScope::Admin)]
#[case::admin_view_only(TraceRole::AdminViewOnly, None, None, None, QueryScope::Admin)]
#[case::team(TraceRole::Other, Some("team-1"), None, None, QueryScope::Team { team_id: "team-1".to_owned() })]
#[case::key(TraceRole::Other, None, Some("hash-1"), None, QueryScope::Key { team_id: String::new(), api_key_hash: "hash-1".to_owned() })]
#[case::project_key(TraceRole::Other, Some("team-1"), Some("hash-1"), Some("project-1"), QueryScope::Key { team_id: "team-1".to_owned(), api_key_hash: "hash-1".to_owned() })]
#[case::project_key_without_team(TraceRole::Other, None, Some("hash-1"), Some("project-1"), QueryScope::Key { team_id: String::new(), api_key_hash: "hash-1".to_owned() })]
fn derives_the_query_scope(
    #[case] role: TraceRole,
    #[case] team_id: Option<&str>,
    #[case] token: Option<&str>,
    #[case] project_id: Option<&str>,
    #[case] expected: QueryScope,
) {
    let request = auth(role, team_id, token, project_id);
    let scope = trace_query_scope(&request).expect("scope");
    assert_eq!(scope, expected);
}

#[test]
fn project_without_a_token_is_rejected() {
    let error = trace_query_scope(&auth(TraceRole::Other, None, None, Some("project-1")))
        .expect_err("project without key");
    assert!(matches!(error, QueryAccessError::ProjectKeyRequired));
}

#[test]
fn no_identity_is_rejected() {
    let error = trace_query_scope(&auth(TraceRole::Other, None, None, None)).expect_err("no scope");
    assert!(matches!(error, QueryAccessError::ScopeRequired));
}

#[test]
fn the_master_key_must_be_configured_and_non_empty() {
    assert!(matches!(
        provide_trace_query_secret(None),
        Err(QueryAccessError::SecretMissing)
    ));
    let empty = SecretString::from(String::new());
    assert!(matches!(
        provide_trace_query_secret(Some(&empty)),
        Err(QueryAccessError::SecretMissing)
    ));
    let configured = SecretString::from("sk-master".to_owned());
    let secret = provide_trace_query_secret(Some(&configured)).expect("secret");
    assert_eq!(secret.expose_secret(), "sk-master");
}

#[test]
fn the_secret_is_checked_before_the_receiver_and_the_scope() {
    let unconfigured =
        provide_trace_query_access(&auth(TraceRole::Other, None, None, None), None, None);
    assert!(matches!(unconfigured, Err(QueryAccessError::SecretMissing)));
    let configured = SecretString::from("sk-master".to_owned());
    let unavailable = provide_trace_query_access(
        &auth(TraceRole::Other, None, None, None),
        None,
        Some(&configured),
    );
    assert!(matches!(unavailable, Err(QueryAccessError::Unavailable)));
}

#[test]
fn an_authenticated_receiver_gets_the_scope_and_the_secret() {
    let configured = SecretString::from("sk-master".to_owned());
    let receiver: Arc<dyn TraceRuntime> = Arc::new(FakeRuntime::new());
    let access = provide_trace_query_access(
        &auth(TraceRole::Other, Some("team-1"), None, None),
        Some(&receiver),
        Some(&configured),
    )
    .expect("access");
    assert_eq!(
        access.scope,
        QueryScope::Team {
            team_id: "team-1".to_owned()
        }
    );
    assert_eq!(access.secret.expose_secret(), "sk-master");
}
