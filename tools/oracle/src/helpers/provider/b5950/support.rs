use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use betterlite_prompts::arguments::json::Json;
use betterlite_provider::sap_auth::deployment::DeploymentConfigurations;
use betterlite_provider::sap_auth::environment::SapEnvironment;
use betterlite_provider::sap_auth::error::ConfigurationLookupError;
use betterlite_provider::sap_auth::token::{
    SapClock, SapTokenTransport, TokenRequest, TokenTransportError,
};
use chrono::{DateTime, Utc};

use super::EnvironmentInput;

impl SapEnvironment for EnvironmentInput {
    fn var(&self, name: &str) -> Option<String> {
        self.vars.get(name).cloned()
    }

    fn file_exists(&self, path: &Path) -> bool {
        path.to_str()
            .is_some_and(|text| self.files.contains_key(text))
    }

    fn read_file(&self, path: &Path) -> Result<String, io::Error> {
        path.to_str()
            .and_then(|text| self.files.get(text))
            .cloned()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }

    fn home_dir(&self) -> PathBuf {
        PathBuf::from(self.home.as_str())
    }
}

pub(super) struct ScriptedClock {
    pub(super) times: Mutex<VecDeque<i64>>,
}

impl SapClock for ScriptedClock {
    fn now(&self) -> DateTime<Utc> {
        let seconds = self
            .times
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(0);
        DateTime::from_timestamp(seconds, 0).unwrap_or(DateTime::UNIX_EPOCH)
    }
}

pub(super) struct Scripted {
    pub(super) responses: VecDeque<String>,
    pub(super) requests: usize,
}

pub(super) struct ScriptedTransport {
    pub(super) script: Mutex<Scripted>,
}

impl SapTokenTransport for ScriptedTransport {
    fn post(&self, _request: &TokenRequest) -> Result<String, TokenTransportError> {
        let mut script = self.script.lock().unwrap_or_else(PoisonError::into_inner);
        script.requests = script.requests.saturating_add(1);
        script.responses.pop_front().ok_or_else(|| {
            TokenTransportError::Message("the case scripted no more token responses".to_owned())
        })
    }
}

pub(super) struct MapConfigurations<'input> {
    pub(super) configurations: &'input BTreeMap<String, Json>,
}

impl DeploymentConfigurations for MapConfigurations<'_> {
    fn configuration(&self, configuration_id: &str) -> Result<Json, ConfigurationLookupError> {
        self.configurations
            .get(configuration_id)
            .cloned()
            .ok_or_else(|| ConfigurationLookupError::Missing(configuration_id.to_owned()))
    }
}
