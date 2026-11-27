#![cfg(test)]

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::future::{Future, ready};
use std::io::Error as IoError;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_db::budget_permission_rows::JsonData;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level};
use tracing_subscriber::layer::{Context as LayerContext, Layer, SubscriberExt as _};

use super::{
    admitted_user_context, build_effective_auth_contexts, own_grant_view, with_parent_span_of,
    with_team_id,
};
use crate::admitted_user::{AdmittedUserError, DelegatedHuman};
use crate::auth_user_lookup::error::UserLookupError;
use crate::resolved_key::auth::{AuthInput, ParentOtelSpan, UserAPIKeyAuth};
use crate::ui_session_identity::{UI_SESSION_TOKEN_TEAM_ID, UiSessionTeams};
use crate::user_roles::UserRole;

fn auth_with(fields: &str) -> UserAPIKeyAuth {
    let fields: BTreeMap<String, JsonData> = serde_json::from_str(fields).unwrap();
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap()
}

fn credential_with(team_id: Option<&str>, user_id: Option<&str>) -> UserAPIKeyAuth {
    let mut fields = BTreeMap::new();
    if let Some(text) = team_id {
        fields.insert("team_id".to_owned(), JsonData::String(text.to_owned()));
    }
    if let Some(text) = user_id {
        fields.insert("user_id".to_owned(), JsonData::String(text.to_owned()));
    }
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap()
}

fn session_for(user_id: &str, fresh: bool) -> UserAPIKeyAuth {
    let mut session = credential_with(Some(UI_SESSION_TOKEN_TEAM_ID), Some(user_id));
    session.extra.requires_fresh_policy = fresh;
    session
}

fn session_with_span(user_id: &str, span: &str) -> UserAPIKeyAuth {
    let mut session = session_for(user_id, false);
    session.extra.parent_otel_span = Some(ParentOtelSpan::Json(JsonData::String(span.to_owned())));
    session
}

fn admitted_auth() -> UserAPIKeyAuth {
    auth_with(
        r#"{"user_id":"test-user","user_role":"internal_user","object_permission_id":"test-permission"}"#,
    )
}

fn span(auth: &UserAPIKeyAuth) -> Option<&JsonData> {
    match &auth.extra.parent_otel_span {
        Some(ParentOtelSpan::Json(value)) => Some(value),
        Some(ParentOtelSpan::Opaque) | None => None,
    }
}

fn secret(value: Option<&SecretString>) -> Option<String> {
    value.map(|held| held.expose_secret().to_owned())
}

fn rich_auth() -> UserAPIKeyAuth {
    auth_with(
        r#"{"team_id":"test-team-1","user_id":"test-user","key_alias":"test-key",
            "api_key":"sk-test-key","user_role":"internal_user",
            "object_permission_id":"test-permission",
            "object_permission":{"object_permission_id":"test-permission","mcp_servers":["test-server"]},
            "parent_otel_span":{"trace_id":"test-trace"}}"#,
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum PortCall {
    Teams(String, bool),
    Human(String, bool),
}

#[derive(Clone, Default)]
struct CallLog(Arc<Mutex<Vec<PortCall>>>);

impl CallLog {
    fn record(&self, call: PortCall) {
        self.0.lock().unwrap().push(call);
    }

    fn calls(&self) -> Vec<PortCall> {
        self.0.lock().unwrap().clone()
    }
}

enum TeamsAnswer {
    Teams(Vec<String>),
    NoUser,
    Failure,
}

struct ScriptedTeams {
    database: bool,
    answer: TeamsAnswer,
    log: CallLog,
}

impl ScriptedTeams {
    fn answering(answer: TeamsAnswer, log: CallLog) -> Self {
        Self {
            database: true,
            answer,
            log,
        }
    }
}

#[async_trait]
impl UiSessionTeams for ScriptedTeams {
    fn has_database(&self) -> bool {
        self.database
    }

    async fn user_teams(
        &self,
        user_id: &str,
        fresh: bool,
    ) -> Result<Option<Vec<String>>, UserLookupError> {
        self.log.record(PortCall::Teams(user_id.to_owned(), fresh));
        match &self.answer {
            TeamsAnswer::Teams(team_ids) => Ok(Some(team_ids.clone())),
            TeamsAnswer::NoUser => Ok(None),
            TeamsAnswer::Failure => Err(UserLookupError::Unavailable(Box::new(IoError::other(
                "the teams read failed",
            )))),
        }
    }
}

enum ReloadAnswer {
    Admitted(UserAPIKeyAuth),
    Refused,
    Outage(String),
}

struct ScriptedHuman {
    answer: ReloadAnswer,
    log: CallLog,
}

impl ScriptedHuman {
    fn admitting(admitted: UserAPIKeyAuth, log: CallLog) -> Self {
        Self {
            answer: ReloadAnswer::Admitted(admitted),
            log,
        }
    }

    fn refusing(log: CallLog) -> Self {
        Self {
            answer: ReloadAnswer::Refused,
            log,
        }
    }
}

impl DelegatedHuman for ScriptedHuman {
    fn reload_admitted_user(
        &self,
        user_id: &str,
        requires_fresh_policy: bool,
    ) -> impl Future<Output = Result<UserAPIKeyAuth, AdmittedUserError>> + Send {
        self.log
            .record(PortCall::Human(user_id.to_owned(), requires_fresh_policy));
        match &self.answer {
            ReloadAnswer::Admitted(admitted) => ready(Ok(admitted.clone())),
            ReloadAnswer::Refused => ready(Err(AdmittedUserError::InvalidCredential)),
            ReloadAnswer::Outage(message) => ready(Err(AdmittedUserError::DatabaseUnavailable {
                message: message.clone(),
            })),
        }
    }
}

#[derive(Default)]
struct EventFields {
    message: Option<String>,
}

impl Visit for EventFields {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_owned());
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{value:?}"));
        }
    }
}

#[derive(Clone, Debug)]
struct LoggedEvent {
    level: Level,
    message: Option<String>,
}

#[derive(Clone, Default)]
struct EventLog(Arc<Mutex<Vec<LoggedEvent>>>);

impl EventLog {
    fn at(&self, level: Level) -> Vec<LoggedEvent> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|event| event.level == level)
            .cloned()
            .collect()
    }
}

impl<S: tracing::Subscriber> Layer<S> for EventLog {
    fn on_event(&self, event: &Event<'_>, _context: LayerContext<'_, S>) {
        let mut fields = EventFields::default();
        event.record(&mut fields);
        self.0.lock().unwrap().push(LoggedEvent {
            level: *event.metadata().level(),
            message: fields.message,
        });
    }
}

fn capture_log() -> (EventLog, DefaultGuard) {
    let log = EventLog::default();
    let guard = set_default(tracing_subscriber::registry().with(log.clone()));
    (log, guard)
}

#[test]
fn copy_with_team_id_changes_only_the_team_id() {
    let mut original = rich_auth();
    original.extra.requires_fresh_policy = true;
    original.extra.mcp_access.mcp_admitted_user_subject = true;
    original.extra.mcp_access.mcp_explicit_grants_only = true;
    original.extra.mcp_session_resource_server_id = Some("test-resource".to_owned());
    original.extra.mcp_toolset_id = Some("test-toolset".to_owned());
    original.extra.provenance.via_virtual_key = true;

    let copied = with_team_id(&original, "test-team-2");

    assert_eq!(copied.view.token.team_id.as_deref(), Some("test-team-2"));
    assert_eq!(original.view.token.team_id.as_deref(), Some("test-team-1"));
    assert_eq!(copied.view.token.user_id, original.view.token.user_id);
    assert_eq!(copied.view.token.key_alias, original.view.token.key_alias);
    assert_eq!(copied.extra.user_role, original.extra.user_role);
    assert_eq!(
        copied.view.token.object_permission,
        original.view.token.object_permission
    );
    assert_eq!(
        copied.view.token.object_permission_id,
        original.view.token.object_permission_id
    );
    assert!(copied.extra.requires_fresh_policy);
    assert!(copied.extra.mcp_access.mcp_admitted_user_subject);
    assert!(copied.extra.mcp_access.mcp_explicit_grants_only);
    assert_eq!(
        copied.extra.mcp_session_resource_server_id.as_deref(),
        Some("test-resource")
    );
    assert_eq!(copied.extra.mcp_toolset_id.as_deref(), Some("test-toolset"));
    assert!(copied.extra.provenance.via_virtual_key);
    assert_eq!(span(&copied), span(&original));
    assert!(span(&copied).is_some());
}

#[test]
fn copy_with_team_id_keeps_the_secrets() {
    let original = rich_auth();

    let copied = with_team_id(&original, "test-team-2");

    assert_eq!(
        secret(copied.view.token.token.as_ref()),
        secret(original.view.token.token.as_ref())
    );
    assert_eq!(
        secret(copied.extra.api_key.0.as_ref()),
        secret(original.extra.api_key.0.as_ref())
    );
    assert!(secret(copied.extra.api_key.0.as_ref()).is_some());
}

#[test]
fn copy_with_parent_span_of_takes_the_sessions_span_and_keeps_the_rest() {
    let mut admitted = admitted_auth();
    admitted.extra.requires_fresh_policy = true;
    let session = session_with_span("test-user", "test-span");

    let copied = with_parent_span_of(&admitted, &session);

    assert_eq!(span(&copied), span(&session));
    assert_eq!(
        span(&copied),
        Some(&JsonData::String("test-span".to_owned()))
    );
    assert_eq!(copied.view.token.user_id.as_deref(), Some("test-user"));
    assert_eq!(copied.view.token.team_id, None);
    assert_eq!(copied.extra.user_role, Some(UserRole::InternalUser));
    assert_eq!(
        copied.view.token.object_permission_id.as_deref(),
        Some("test-permission")
    );
    assert!(copied.extra.requires_fresh_policy);
    assert!(admitted.extra.parent_otel_span.is_none());
}

#[test]
fn copy_with_parent_span_of_replaces_an_earlier_span() {
    let mut admitted = admitted_auth();
    admitted.extra.parent_otel_span = Some(ParentOtelSpan::Json(JsonData::String(
        "test-own".to_owned(),
    )));
    let session = session_with_span("test-user", "test-session-span");

    let copied = with_parent_span_of(&admitted, &session);

    assert_eq!(
        span(&copied),
        Some(&JsonData::String("test-session-span".to_owned()))
    );
    assert_eq!(
        span(&admitted),
        Some(&JsonData::String("test-own".to_owned()))
    );
}

#[rstest]
#[case::own_permission(
    r#"{"user_id":"test-user","object_permission_id":"test-permission","object_permission":{"object_permission_id":"test-permission"}}"#,
    true,
    Some("test-permission")
)]
#[case::id_without_the_table(
    r#"{"user_id":"test-user","object_permission_id":"test-permission"}"#,
    false,
    Some("test-permission")
)]
#[case::table_without_the_id(
    r#"{"user_id":"test-user","object_permission":{"object_permission_id":"test-permission"}}"#,
    true,
    None
)]
#[case::neither(r#"{"user_id":"test-user"}"#, false, None)]
fn grant_view_reports_the_object_permission_presence_and_id(
    #[case] fields: &str,
    #[case] has_object_permission: bool,
    #[case] object_permission_id: Option<&str>,
) {
    let auth = auth_with(fields);

    let view = own_grant_view(&auth);

    assert_eq!(view.has_object_permission, has_object_permission);
    assert_eq!(view.object_permission_id, object_permission_id);
}

#[rstest]
#[case::proxy_admin(Some(UserRole::ProxyAdmin), true)]
#[case::proxy_admin_viewer(Some(UserRole::ProxyAdminViewer), true)]
#[case::org_admin(Some(UserRole::OrgAdmin), false)]
#[case::internal_user(Some(UserRole::InternalUser), false)]
#[case::internal_user_viewer(Some(UserRole::InternalUserViewer), false)]
#[case::team(Some(UserRole::Team), false)]
#[case::customer(Some(UserRole::Customer), false)]
#[case::no_role(None, false)]
fn grant_view_admin_view_is_proxy_admin_and_proxy_admin_viewer_only(
    #[case] role: Option<UserRole>,
    #[case] has_admin_view: bool,
) {
    let mut auth = auth_with(r#"{"user_id":"test-user"}"#);
    auth.extra.user_role = role;

    assert_eq!(own_grant_view(&auth).has_admin_view, has_admin_view);
}

#[rstest]
#[case::another_team(Some("test-team-1"), Some("test-user"))]
#[case::no_team(None, Some("test-user"))]
#[case::dashboard_without_a_user(Some(UI_SESSION_TOKEN_TEAM_ID), Some(""))]
#[case::dashboard_without_a_user_id(Some(UI_SESSION_TOKEN_TEAM_ID), None)]
#[tokio::test]
async fn admitted_context_is_none_for_a_credential_that_is_not_a_session(
    #[case] team_id: Option<&str>,
    #[case] user_id: Option<&str>,
) {
    let credential = credential_with(team_id, user_id);
    let log = CallLog::default();
    let human = ScriptedHuman::admitting(admitted_auth(), log.clone());

    assert!(admitted_user_context(&credential, &human).await.is_none());
    assert!(log.calls().is_empty());
}

#[rstest]
#[case::cached(false)]
#[case::fresh(true)]
#[tokio::test]
async fn admitted_context_reloads_the_session_user_with_its_fresh_flag(#[case] fresh: bool) {
    let session = session_for("test-user", fresh);
    let log = CallLog::default();
    let human = ScriptedHuman::admitting(admitted_auth(), log.clone());

    let context = admitted_user_context(&session, &human).await.unwrap();

    assert_eq!(
        log.calls(),
        vec![PortCall::Human("test-user".to_owned(), fresh)]
    );
    assert_eq!(context.view.token.user_id.as_deref(), Some("test-user"));
    assert_eq!(context.extra.user_role, Some(UserRole::InternalUser));
}

#[tokio::test]
async fn admitted_context_puts_the_sessions_span_on_the_reloaded_user() {
    let session = session_with_span("test-user", "test-span");
    let human = ScriptedHuman::admitting(admitted_auth(), CallLog::default());

    let context = admitted_user_context(&session, &human).await.unwrap();

    assert_eq!(span(&context), span(&session));
    assert_eq!(
        span(&context),
        Some(&JsonData::String("test-span".to_owned()))
    );
    assert_eq!(context.view.token.user_id.as_deref(), Some("test-user"));
    assert_eq!(context.view.token.team_id, None);
    assert_eq!(context.extra.user_role, Some(UserRole::InternalUser));
}

#[rstest]
#[case::refused(
    ReloadAnswer::Refused,
    "MCP dashboard session: admitted-subject reload failed for test-user: Invalid or expired credential"
)]
#[case::outage(
    ReloadAnswer::Outage("the auth database is unavailable".to_owned()),
    "MCP dashboard session: admitted-subject reload failed for test-user: the auth database is unavailable"
)]
#[tokio::test]
async fn admitted_context_is_none_and_warns_when_the_reload_fails(
    #[case] answer: ReloadAnswer,
    #[case] expected: &str,
) {
    let (log, _guard) = capture_log();
    let session = session_for("test-user", false);
    let human = ScriptedHuman {
        answer,
        log: CallLog::default(),
    };

    assert!(admitted_user_context(&session, &human).await.is_none());

    let warnings = log.at(Level::WARN);
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].message.as_deref(), Some(expected));
}

#[tokio::test]
async fn contexts_keep_the_team_copies_when_the_admitted_context_exists() {
    let session = session_for("test-user", false);
    let log = CallLog::default();
    let teams = ScriptedTeams::answering(
        TeamsAnswer::Teams(vec!["test-team-2".to_owned(), "test-team-1".to_owned()]),
        log.clone(),
    );
    let human = ScriptedHuman::admitting(admitted_auth(), log.clone());

    let contexts = build_effective_auth_contexts(&session, &teams, &human).await;

    assert_eq!(contexts.len(), 3);
    assert_eq!(
        contexts[0].view.token.team_id.as_deref(),
        Some("test-team-2")
    );
    assert_eq!(
        contexts[1].view.token.team_id.as_deref(),
        Some("test-team-1")
    );
    assert_eq!(contexts[0].view.token.user_id.as_deref(), Some("test-user"));
    assert_eq!(contexts[1].view.token.user_id.as_deref(), Some("test-user"));
    assert_eq!(contexts[2].view.token.user_id.as_deref(), Some("test-user"));
    assert_eq!(contexts[2].view.token.team_id, None);
    assert_eq!(contexts[2].extra.user_role, Some(UserRole::InternalUser));
}

#[rstest]
#[case::no_user_row(TeamsAnswer::NoUser)]
#[case::no_teams(TeamsAnswer::Teams(Vec::new()))]
#[tokio::test]
async fn contexts_without_resolved_teams_are_the_session_then_the_admitted_user(
    #[case] answer: TeamsAnswer,
) {
    let mut session = session_for("test-user", true);
    session.extra.mcp_session_resource_server_id = Some("test-resource".to_owned());
    let teams = ScriptedTeams::answering(answer, CallLog::default());
    let human = ScriptedHuman::admitting(admitted_auth(), CallLog::default());

    let contexts = build_effective_auth_contexts(&session, &teams, &human).await;

    assert_eq!(contexts.len(), 2);
    assert_eq!(
        serde_json::to_string(&contexts[0]).unwrap(),
        serde_json::to_string(&session).unwrap()
    );
    assert!(contexts[0].extra.requires_fresh_policy);
    assert_eq!(
        contexts[0].extra.mcp_session_resource_server_id.as_deref(),
        Some("test-resource")
    );
    assert_eq!(contexts[1].extra.user_role, Some(UserRole::InternalUser));
}

#[tokio::test]
async fn contexts_without_teams_or_an_admitted_user_are_the_session_alone() {
    let session = session_for("test-user", false);
    let teams = ScriptedTeams::answering(TeamsAnswer::NoUser, CallLog::default());
    let human = ScriptedHuman::refusing(CallLog::default());

    let contexts = build_effective_auth_contexts(&session, &teams, &human).await;

    assert_eq!(contexts.len(), 1);
    assert_eq!(
        serde_json::to_string(&contexts[0]).unwrap(),
        serde_json::to_string(&session).unwrap()
    );
}

#[tokio::test]
async fn contexts_for_a_non_session_credential_are_the_credential_alone_without_port_calls() {
    let credential = credential_with(Some("test-team-1"), Some("test-user"));
    let log = CallLog::default();
    let teams = ScriptedTeams::answering(
        TeamsAnswer::Teams(vec!["test-team-2".to_owned()]),
        log.clone(),
    );
    let human = ScriptedHuman::admitting(admitted_auth(), log.clone());

    let contexts = build_effective_auth_contexts(&credential, &teams, &human).await;

    assert_eq!(contexts.len(), 1);
    assert_eq!(
        serde_json::to_string(&contexts[0]).unwrap(),
        serde_json::to_string(&credential).unwrap()
    );
    assert!(log.calls().is_empty());
}

#[tokio::test]
async fn contexts_keep_the_team_copies_when_the_admitted_reload_fails() {
    let session = session_for("test-user", false);
    let teams = ScriptedTeams::answering(
        TeamsAnswer::Teams(vec!["test-team-1".to_owned()]),
        CallLog::default(),
    );
    let human = ScriptedHuman::refusing(CallLog::default());

    let contexts = build_effective_auth_contexts(&session, &teams, &human).await;

    assert_eq!(contexts.len(), 1);
    assert_eq!(
        contexts[0].view.token.team_id.as_deref(),
        Some("test-team-1")
    );
}

#[tokio::test]
async fn contexts_team_copies_carry_the_sessions_server_only_fields() {
    let mut session = session_for("test-user", true);
    session.extra.mcp_session_resource_server_id = Some("test-resource".to_owned());
    session.extra.parent_otel_span = Some(ParentOtelSpan::Json(JsonData::String(
        "test-span".to_owned(),
    )));
    let teams = ScriptedTeams::answering(
        TeamsAnswer::Teams(vec!["test-team-1".to_owned()]),
        CallLog::default(),
    );
    let human = ScriptedHuman::refusing(CallLog::default());

    let contexts = build_effective_auth_contexts(&session, &teams, &human).await;

    assert_eq!(contexts.len(), 1);
    assert!(contexts[0].extra.requires_fresh_policy);
    assert_eq!(
        contexts[0].extra.mcp_session_resource_server_id.as_deref(),
        Some("test-resource")
    );
    assert_eq!(span(&contexts[0]), span(&session));
    assert_eq!(contexts[0].view.token.user_id.as_deref(), Some("test-user"));
}

#[tokio::test]
async fn contexts_resolve_the_teams_before_the_admitted_reload() {
    let session = session_for("test-user", true);
    let log = CallLog::default();
    let teams = ScriptedTeams::answering(
        TeamsAnswer::Teams(vec!["test-team-1".to_owned()]),
        log.clone(),
    );
    let human = ScriptedHuman::admitting(admitted_auth(), log.clone());

    build_effective_auth_contexts(&session, &teams, &human).await;

    assert_eq!(
        log.calls(),
        vec![
            PortCall::Teams("test-user".to_owned(), true),
            PortCall::Human("test-user".to_owned(), true),
        ]
    );
}
