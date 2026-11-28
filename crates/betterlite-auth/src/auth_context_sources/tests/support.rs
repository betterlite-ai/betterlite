use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::auth_context_sources::Settings;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use tracing::Level;
use tracing::subscriber::DefaultGuard;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::util::SubscriberInitExt as _;

use crate::auth_context_sources::claude::RouterNames;
use crate::auth_context_sources::router_settings::TeamRouterSettings;
use crate::auth_request_models::catalog::{
    ManagedIdDecoders, ManagedIdError, ManagedIdFamily, ModelCatalog,
};
use crate::auth_team_lookup::error::TeamLookupError;

#[must_use]
pub(crate) fn object(text: &str) -> JsonObject {
    serde_json::from_str(text).expect("the fixture parses")
}

#[must_use]
pub(crate) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture parses")
}

#[must_use]
pub(crate) fn set_of(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

// The production config loader resolves the SSO settings from the environment it is handed.
#[must_use]
pub(crate) fn env_settings(environment: &[(&str, &str)]) -> Settings {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    ConfigLoader { lookup }
        .load("")
        .expect("the empty config loads")
        .sections
        .auth_context_sources
}

#[must_use]
pub(crate) fn team_columns(text: &str) -> BTreeMap<String, JsonData> {
    serde_json::from_str(text).expect("the fixture parses")
}

#[derive(Debug, Default)]
pub(crate) struct NoDecoders;

impl ManagedIdDecoders for NoDecoders {
    fn decode_family(
        &self,
        _family: ManagedIdFamily,
        _resource_id: &str,
        _catalog: Option<&dyn ModelCatalog>,
        _candidates: &mut Vec<String>,
    ) -> Result<(), ManagedIdError> {
        Ok(())
    }
}

#[derive(Debug, Default)]
pub(crate) struct FakeRouter {
    pub(crate) alias_groups: BTreeSet<String>,
    pub(crate) model_ids: BTreeSet<String>,
    pub(crate) candidates: BTreeMap<String, Vec<String>>,
    pub(crate) asked: Mutex<Vec<String>>,
}

impl FakeRouter {
    #[must_use]
    pub(crate) fn new(
        alias_groups: BTreeSet<String>,
        model_ids: BTreeSet<String>,
        candidates: BTreeMap<String, Vec<String>>,
    ) -> Self {
        Self {
            alias_groups,
            model_ids,
            candidates,
            asked: Mutex::new(Vec::new()),
        }
    }

    #[must_use]
    pub(crate) fn asked(&self) -> Vec<String> {
        self.asked
            .lock()
            .expect("the recorder is not poisoned")
            .clone()
    }
}

impl RouterNames for FakeRouter {
    fn model_group_alias_contains(&self, name: &str) -> bool {
        self.asked
            .lock()
            .expect("the recorder is not poisoned")
            .push(name.to_owned());
        self.alias_groups.contains(name)
    }

    fn has_model_id(&self, name: &str) -> bool {
        self.asked
            .lock()
            .expect("the recorder is not poisoned")
            .push(name.to_owned());
        self.model_ids.contains(name)
    }

    fn candidate_model_ids_for_route(&self, name: &str, _team_id: Option<&str>) -> Vec<String> {
        self.asked
            .lock()
            .expect("the recorder is not poisoned")
            .push(name.to_owned());
        self.candidates.get(name).cloned().unwrap_or_default()
    }
}

#[derive(Debug)]
pub(crate) enum TeamAnswer {
    Settings(BTreeMap<String, JsonData>),
    Absent,
    Failure,
}

#[derive(Debug)]
pub(crate) struct FakeTeams {
    answer: TeamAnswer,
    calls: Mutex<Vec<String>>,
}

impl FakeTeams {
    pub(crate) fn new(answer: TeamAnswer) -> Self {
        Self {
            answer,
            calls: Mutex::new(Vec::new()),
        }
    }

    #[must_use]
    pub(crate) fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .expect("the recorder is not poisoned")
            .clone()
    }
}

#[async_trait]
impl TeamRouterSettings for FakeTeams {
    async fn router_settings(
        &self,
        team_id: &str,
    ) -> Result<Option<BTreeMap<String, JsonData>>, TeamLookupError> {
        self.calls
            .lock()
            .expect("the recorder is not poisoned")
            .push(team_id.to_owned());
        match &self.answer {
            TeamAnswer::Settings(columns) => Ok(Some(columns.clone())),
            TeamAnswer::Absent => Ok(None),
            TeamAnswer::Failure => Err(TeamLookupError::NoDatabase),
        }
    }
}

// A log capture for the tests that assert on a debug line; the fmt layer writes into memory.
#[derive(Clone, Default)]
pub(crate) struct Logs(Arc<Mutex<Vec<u8>>>);

impl Logs {
    pub(crate) fn capture() -> (Self, DefaultGuard) {
        let logs = Self::default();
        let guard = tracing_subscriber::fmt()
            .with_max_level(Level::DEBUG)
            .with_writer(logs.clone())
            .with_ansi(false)
            .set_default();
        (logs, guard)
    }

    pub(crate) fn text(&self) -> String {
        String::from_utf8(
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        )
        .expect("the captured log output is UTF-8")
    }
}

impl io::Write for Logs {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut logged = self
            .0
            .lock()
            .map_err(|_poisoned| io::Error::other("log buffer poisoned"))?;
        logged.extend_from_slice(buf);
        drop(logged);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for Logs {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}
