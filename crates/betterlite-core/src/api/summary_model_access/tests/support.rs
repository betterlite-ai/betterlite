use std::collections::{BTreeMap, VecDeque};
use std::fmt::Debug;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_auth::auth_model_patterns::denial::model_access_object_name;
use betterlite_auth::auth_model_patterns::view::{DeploymentView, ModelAccessView};
use betterlite_auth::identity_grants::resolver::user_models;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::resource_rows::project::ProjectTable;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;
use tracing::field::{Field, Visit};
use tracing::subscriber::set_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use crate::api::summary_model_access::access::{ProxyGlobals, check_summary_model_access};
use crate::api::summary_model_access::checks::{ObjectCall, SummaryModelChecks};
use crate::api::summary_model_access::error::PolicyFailure;
use crate::api::summary_model_access::repositories::SummaryAccessRepositories;

pub(super) const SUM: &str = "sum";

// every scope the check reads, with a distinct allowlist behind each one
pub(super) const EVERY_SCOPE: &str = r#"{
    "models": ["key-model"],
    "team_models": ["team-model"],
    "team_model_aliases": {"sum": "team-model"},
    "team_id": "team-1",
    "user_id": "user-1",
    "project_id": "project-1"
}"#;

pub(super) fn principal(text: &str) -> UserAPIKeyAuth {
    let map: BTreeMap<String, JsonData> = serde_json::from_str(text).unwrap();
    UserAPIKeyAuth::construct(AuthInput::Dict(map)).unwrap()
}

pub(super) fn user_row(models: &[&str]) -> LiteLLMUserTable {
    let mut row: LiteLLMUserTable = serde_json::from_str(r#"{"user_id": "user-1"}"#).unwrap();
    row.models = models
        .iter()
        .map(|name| JsonData::String((*name).to_owned()))
        .collect();
    row
}

pub(super) fn project_row(models: &[&str]) -> ProjectTable {
    let mut row: ProjectTable = serde_json::from_str(r#"{"project_id": "project-1"}"#).unwrap();
    row.models = models.iter().map(|name| (*name).to_owned()).collect();
    row
}

// None is a membership without a budget table, and an empty slice is an empty allowlist
pub(super) fn membership_row(allowed: Option<&[&str]>) -> LiteLLMTeamMembership {
    let Some(models) = allowed else {
        return serde_json::from_str(r#"{"user_id": "user-1", "team_id": "team-1"}"#).unwrap();
    };
    let listed = serde_json::to_string(models).unwrap();
    serde_json::from_str(&format!(
        r#"{{"user_id": "user-1", "team_id": "team-1", "litellm_budget_table": {{"allowed_models": {listed}}}}}"#
    ))
    .unwrap()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Call {
    Object {
        object_type: &'static str,
        model: String,
        models: Vec<String>,
        team_model_aliases: BTreeMap<String, String>,
        team_id: Option<String>,
        router: bool,
    },
    User {
        model: String,
        user_models: Vec<String>,
        router: bool,
    },
    Project {
        model: String,
        project_models: Vec<String>,
        router: bool,
    },
    LookupUser(String, bool),
    LookupProject(String),
    LookupMembership(String, String),
}

fn object_call(
    object_type: &'static str,
    models: &[&str],
    aliases: &BTreeMap<String, String>,
) -> Call {
    Call::Object {
        object_type,
        model: SUM.to_owned(),
        models: models.iter().map(|name| (*name).to_owned()).collect(),
        team_model_aliases: aliases.clone(),
        team_id: Some("team-1".to_owned()),
        router: true,
    }
}

// the calls one open-every-scope run makes, in source order
pub(super) fn every_scope_journal() -> Vec<Call> {
    let aliases = BTreeMap::from([(SUM.to_owned(), "team-model".to_owned())]);
    vec![
        object_call("key", &["key-model"], &aliases),
        object_call("team", &["team-model"], &aliases),
        // the summary check reads the user row and never creates one
        Call::LookupUser("user-1".to_owned(), false),
        Call::User {
            model: SUM.to_owned(),
            user_models: vec!["key-model".to_owned()],
            router: true,
        },
        Call::LookupProject("project-1".to_owned()),
        Call::Project {
            model: SUM.to_owned(),
            project_models: vec!["team-model".to_owned()],
            router: true,
        },
        Call::LookupMembership("user-1".to_owned(), "team-1".to_owned()),
        object_call("team", &["member-model"], &aliases),
    ]
}

pub(super) fn journal_between(start: usize, end: usize) -> Vec<Call> {
    let mut calls = every_scope_journal();
    calls.truncate(end);
    calls.split_off(start)
}

#[derive(Clone, Default)]
pub(super) struct Journal(Arc<Mutex<Vec<Call>>>);

impl Journal {
    fn record(&self, call: Call) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
    }

    pub(super) fn calls(&self) -> Vec<Call> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Outcome {
    Allow,
    Deny,
    Broken,
}

pub(super) struct FakeChecks {
    journal: Journal,
    outcomes: Mutex<VecDeque<Outcome>>,
}

impl FakeChecks {
    fn with(journal: &Journal, outcomes: &[Outcome]) -> Self {
        Self {
            journal: journal.clone(),
            outcomes: Mutex::new(outcomes.iter().copied().collect()),
        }
    }

    // an empty script allows, so an extra call still shows up in the journal
    fn next(&self) -> Outcome {
        self.outcomes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(Outcome::Allow)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("broken")]
pub(super) struct Broken;

const fn verdict(outcome: Outcome) -> Result<(), PolicyFailure<Broken>> {
    match outcome {
        Outcome::Allow => Ok(()),
        Outcome::Deny => Err(PolicyFailure::Denied),
        Outcome::Broken => Err(PolicyFailure::Unexpected(Broken)),
    }
}

// the three fake checks differ only in what they journal, so one macro writes all three
macro_rules! journaled_check {
    (
        $method:ident,
        ($($argument:ident: $argument_type:ty),* $(,)?),
        $record:expr $(,)?
    ) => {
        fn $method(
            &self,
            $($argument: $argument_type,)*
        ) -> Result<(), PolicyFailure<Broken>> {
            self.journal.record($record);
            verdict(self.next())
        }
    };
}

impl SummaryModelChecks for FakeChecks {
    type Failure = Broken;

    journaled_check!(
        object_call,
        (router: Option<&dyn ModelAccessView>, call: &ObjectCall<'_>),
        Call::Object {
            object_type: model_access_object_name(call.object_type),
            model: call.model.to_owned(),
            models: call.models.to_vec(),
            team_model_aliases: call.team_model_aliases.clone(),
            team_id: call.team_id.map(str::to_owned),
            router: router.is_some(),
        },
    );

    journaled_check!(
        user_call,
        (router: Option<&dyn ModelAccessView>, model: &str, user: &LiteLLMUserTable),
        Call::User {
            model: model.to_owned(),
            user_models: user_models(user),
            router: router.is_some(),
        },
    );

    journaled_check!(
        project_call,
        (router: Option<&dyn ModelAccessView>, model: &str, project: &ProjectTable),
        Call::Project {
            model: model.to_owned(),
            project_models: project.models.clone(),
            router: router.is_some(),
        },
    );
}

#[derive(Debug, thiserror::Error)]
#[error("offline")]
pub(super) struct Offline;

pub(super) enum Lookup<T> {
    Found(T),
    Missing,
    Failed,
}

pub(super) struct FakeRepositories {
    pub(super) journal: Journal,
    pub(super) user: Lookup<LiteLLMUserTable>,
    pub(super) project: Lookup<ProjectTable>,
    pub(super) membership: Lookup<LiteLLMTeamMembership>,
}

impl FakeRepositories {
    fn with(journal: &Journal) -> Self {
        Self {
            journal: journal.clone(),
            user: Lookup::Missing,
            project: Lookup::Missing,
            membership: Lookup::Missing,
        }
    }
}

// each fake lookup differs only in its row type and journal entry, so one macro writes all three
macro_rules! fake_lookup {
    (
        $method:ident,
        ($($argument:ident: $argument_type:ty),* $(,)?),
        $row:ty,
        $lookup:ident,
        $record:expr $(,)?
    ) => {
        fn $method(
            &self,
            $($argument: $argument_type,)*
        ) -> impl Future<Output = Result<Option<$row>, Offline>> + Send {
            self.journal.record($record);
            ready(match &self.$lookup {
                Lookup::Found(row) => Ok(Some(row.clone())),
                Lookup::Missing => Ok(None),
                Lookup::Failed => Err(Offline),
            })
        }
    };
}

impl SummaryAccessRepositories for FakeRepositories {
    type Failure = Offline;

    fake_lookup!(
        user,
        (user_id: &str, user_id_upsert: bool),
        LiteLLMUserTable,
        user,
        Call::LookupUser(user_id.to_owned(), user_id_upsert),
    );

    fake_lookup!(
        project,
        (project_id: &str),
        ProjectTable,
        project,
        Call::LookupProject(project_id.to_owned()),
    );

    fake_lookup!(
        membership,
        (user_id: &str, team_id: &str),
        LiteLLMTeamMembership,
        membership,
        Call::LookupMembership(user_id.to_owned(), team_id.to_owned()),
    );
}

pub(super) struct Harness {
    pub journal: Journal,
    pub checks: FakeChecks,
    pub repositories: FakeRepositories,
    pub router: DeploymentView,
}

impl Harness {
    // a database behind every lookup, with no row behind any of them
    pub(super) fn empty(outcomes: &[Outcome]) -> Self {
        let journal = Journal::default();
        Self {
            checks: FakeChecks::with(&journal, outcomes),
            repositories: FakeRepositories::with(&journal),
            journal,
            router: DeploymentView::default(),
        }
    }

    // every lookup finds a row, and the checks follow the given script
    pub(super) fn scripted(outcomes: &[Outcome]) -> Self {
        let mut harness = Self::empty(outcomes);
        harness.repositories.user = Lookup::Found(user_row(&["key-model"]));
        harness.repositories.project = Lookup::Found(project_row(&["team-model"]));
        harness.repositories.membership = Lookup::Found(membership_row(Some(&["member-model"])));
        harness
    }

    // every lookup finds a row, and each scope allows
    pub(super) fn full() -> Self {
        Self::scripted(&[Outcome::Allow; 5])
    }

    pub(super) fn globals(&self, database: bool) -> ProxyGlobals<'_, FakeChecks, FakeRepositories> {
        let router: Option<&dyn ModelAccessView> = Some(&self.router);
        ProxyGlobals {
            database,
            router,
            checks: &self.checks,
            repositories: &self.repositories,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Diagnostic {
    pub level: Level,
    pub message: String,
}

pub(super) fn warn(message: &str) -> Diagnostic {
    Diagnostic {
        level: Level::WARN,
        message: message.to_owned(),
    }
}

pub(super) fn debug(message: &str) -> Diagnostic {
    Diagnostic {
        level: Level::DEBUG,
        message: message.to_owned(),
    }
}

#[derive(Clone, Default)]
pub(super) struct Diagnostics(Arc<Mutex<Vec<Diagnostic>>>);

impl Diagnostics {
    fn lines(&self) -> Vec<Diagnostic> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Diagnostics {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let level = *event.metadata().level();
        if level != Level::WARN && level != Level::DEBUG {
            return;
        }
        let mut message = Message::default();
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Diagnostic {
                level,
                message: message.0,
            });
    }
}

pub(super) struct Run {
    pub allowed: bool,
    pub calls: Vec<Call>,
    pub diagnostics: Vec<Diagnostic>,
}

pub(super) async fn run(
    auth: Option<&UserAPIKeyAuth>,
    globals: Option<&ProxyGlobals<'_, FakeChecks, FakeRepositories>>,
    journal: &Journal,
) -> Run {
    let diagnostics = Diagnostics::default();
    let guard = set_default(registry().with(diagnostics.clone()));
    let allowed = check_summary_model_access(auth, SUM, globals)
        .await
        .unwrap();
    drop(guard);
    Run {
        allowed,
        calls: journal.calls(),
        diagnostics: diagnostics.lines(),
    }
}
