#![cfg(test)]

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::io::Error as IoError;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_db::budget_permission_rows::JsonData;
use rstest::rstest;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level};
use tracing_subscriber::layer::{Context as LayerContext, Layer, SubscriberExt as _};

use super::{
    UI_SESSION_TOKEN_TEAM_ID, UiSessionTeams, is_ui_session_credential, resolve_ui_session_team_ids,
};
use crate::auth_user_lookup::error::UserLookupError;
use crate::resolved_key::auth::{AuthInput, UserAPIKeyAuth};

fn token(team_id: Option<&str>, user_id: Option<&str>, fresh: bool) -> UserAPIKeyAuth {
    let mut fields = BTreeMap::new();
    for (name, value) in [("team_id", team_id), ("user_id", user_id)] {
        if let Some(text) = value {
            fields.insert(name.to_owned(), JsonData::String(text.to_owned()));
        }
    }
    let mut auth = UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap();
    auth.extra.requires_fresh_policy = fresh;
    auth
}

fn session_token(user_id: &str, fresh: bool) -> UserAPIKeyAuth {
    token(Some(UI_SESSION_TOKEN_TEAM_ID), Some(user_id), fresh)
}

enum Answer {
    Teams(Vec<String>),
    NoUser,
    Failure,
}

struct ScriptedTeams {
    database: bool,
    answer: Answer,
    calls: Mutex<Vec<(String, bool)>>,
}

impl ScriptedTeams {
    fn answering(answer: Answer) -> Self {
        Self {
            database: true,
            answer,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<(String, bool)> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
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
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((user_id.to_owned(), fresh));
        match &self.answer {
            Answer::Teams(teams) => Ok(Some(teams.clone())),
            Answer::NoUser => Ok(None),
            Answer::Failure => Err(UserLookupError::Unavailable(Box::new(IoError::other(
                "the case lookup failed",
            )))),
        }
    }
}

#[derive(Default)]
struct EventFields {
    message: Option<String>,
    error: Option<String>,
}

impl Visit for EventFields {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_owned());
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        let text = format!("{value:?}");
        match field.name() {
            "message" => self.message = Some(text),
            "error" => self.error = Some(text),
            _ => {}
        }
    }
}

#[derive(Clone, Debug)]
struct Logged {
    level: Level,
    message: Option<String>,
    error: Option<String>,
}

#[derive(Clone, Default)]
struct EventLog(Arc<Mutex<Vec<Logged>>>);

impl EventLog {
    fn at(&self, level: Level) -> Vec<Logged> {
        let events = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        events
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
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Logged {
                level: *event.metadata().level(),
                message: fields.message,
                error: fields.error,
            });
    }
}

fn capture_log() -> (EventLog, DefaultGuard) {
    let log = EventLog::default();
    let guard = set_default(tracing_subscriber::registry().with(log.clone()));
    (log, guard)
}

#[rstest]
#[case::dashboard_team_and_user(Some(UI_SESSION_TOKEN_TEAM_ID), Some("user-1"), true)]
#[case::dashboard_team_empty_user(Some(UI_SESSION_TOKEN_TEAM_ID), Some(""), false)]
#[case::dashboard_team_missing_user(Some(UI_SESSION_TOKEN_TEAM_ID), None, false)]
#[case::another_team(Some("team-1"), Some("user-1"), false)]
#[case::no_team(None, Some("user-1"), false)]
#[case::dashboard_team_in_another_case(Some("LiteLLM-Dashboard"), Some("user-1"), false)]
#[case::dashboard_team_with_whitespace(Some(" litellm-dashboard"), Some("user-1"), false)]
fn the_session_credential_is_the_dashboard_team_with_a_user(
    #[case] team_id: Option<&str>,
    #[case] user_id: Option<&str>,
    #[case] expected: bool,
) {
    assert_eq!(
        is_ui_session_credential(&token(team_id, user_id, false)),
        expected
    );
}

#[rstest]
#[case::another_team(Some("team-1"), Some("user-1"))]
#[case::no_team(None, Some("user-1"))]
#[case::dashboard_team_without_a_user(Some(UI_SESSION_TOKEN_TEAM_ID), Some(""))]
#[tokio::test]
async fn a_credential_that_is_not_a_session_resolves_nothing_without_a_port_call(
    #[case] team_id: Option<&str>,
    #[case] user_id: Option<&str>,
) {
    let auth = token(team_id, user_id, false);
    let port = ScriptedTeams::answering(Answer::Teams(vec!["team-1".to_owned()]));
    assert_eq!(
        resolve_ui_session_team_ids(&auth, &port).await,
        Vec::<String>::new()
    );
    assert!(port.calls().is_empty());
}

#[tokio::test]
async fn a_missing_database_resolves_nothing_and_logs_a_debug_line() {
    let (log, _guard) = capture_log();
    let auth = session_token("user-1", false);
    let mut port = ScriptedTeams::answering(Answer::Teams(vec!["team-1".to_owned()]));
    port.database = false;
    assert_eq!(
        resolve_ui_session_team_ids(&auth, &port).await,
        Vec::<String>::new()
    );
    assert!(port.calls().is_empty());
    let debug = log.at(Level::DEBUG);
    assert_eq!(debug.len(), 1);
    assert_eq!(
        debug[0].message.as_deref(),
        Some("Cannot resolve UI session team ids without DB access")
    );
}

#[tokio::test]
async fn a_user_read_failure_resolves_nothing_and_warns_with_the_error() {
    let (log, _guard) = capture_log();
    let auth = session_token("user-1", false);
    let port = ScriptedTeams::answering(Answer::Failure);
    assert_eq!(
        resolve_ui_session_team_ids(&auth, &port).await,
        Vec::<String>::new()
    );
    assert_eq!(port.calls().len(), 1);
    let warnings = log.at(Level::WARN);
    assert_eq!(warnings.len(), 1);
    assert_eq!(
        warnings[0].message.as_deref(),
        Some("Failed to load teams for UI session token user.")
    );
    let error = warnings[0].error.as_deref().unwrap_or_default();
    assert!(error.contains("the case lookup failed"), "{error}");
}

#[rstest]
#[case::no_user(Answer::NoUser)]
#[case::no_teams(Answer::Teams(Vec::new()))]
#[tokio::test]
async fn a_user_without_teams_resolves_nothing(#[case] answer: Answer) {
    let (log, _guard) = capture_log();
    let auth = session_token("user-1", false);
    let port = ScriptedTeams::answering(answer);
    assert_eq!(
        resolve_ui_session_team_ids(&auth, &port).await,
        Vec::<String>::new()
    );
    assert_eq!(port.calls().len(), 1);
    assert!(log.at(Level::WARN).is_empty());
}

#[tokio::test]
async fn the_teams_come_back_once_each_in_first_seen_order() {
    let auth = session_token("user-1", false);
    let port = ScriptedTeams::answering(Answer::Teams(vec![
        "team-2".to_owned(),
        UI_SESSION_TOKEN_TEAM_ID.to_owned(),
        String::new(),
        "team-2".to_owned(),
        "team-1".to_owned(),
    ]));
    assert_eq!(
        resolve_ui_session_team_ids(&auth, &port).await,
        vec![
            "team-2".to_owned(),
            UI_SESSION_TOKEN_TEAM_ID.to_owned(),
            "team-1".to_owned()
        ]
    );
}

#[rstest]
#[case::fresh(true)]
#[case::cached(false)]
#[tokio::test]
async fn the_user_is_read_once_with_the_auth_user_and_fresh_flag(#[case] fresh: bool) {
    let auth = session_token("user-1", fresh);
    let port = ScriptedTeams::answering(Answer::Teams(vec!["team-1".to_owned()]));
    assert_eq!(
        resolve_ui_session_team_ids(&auth, &port).await,
        vec!["team-1".to_owned()]
    );
    assert_eq!(port.calls(), vec![("user-1".to_owned(), fresh)]);
}
