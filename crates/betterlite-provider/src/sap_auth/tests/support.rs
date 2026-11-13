use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use betterlite_prompts::arguments::json::Json;
use chrono::{DateTime, Utc};

use crate::sap_auth::credentials::Credentials;
use crate::sap_auth::deployment::DeploymentConfigurations;
use crate::sap_auth::environment::SapEnvironment;
use crate::sap_auth::error::ConfigurationLookupError;
use crate::sap_auth::token::{SapClock, SapTokenTransport, TokenRequest, TokenTransportError};

#[derive(Debug, Default)]
pub(super) struct FakeEnvironment {
    pub(super) vars: BTreeMap<String, String>,
    pub(super) files: BTreeMap<PathBuf, String>,
    pub(super) unreadable: BTreeSet<PathBuf>,
    pub(super) home: PathBuf,
}

impl FakeEnvironment {
    pub(super) fn with_home(mut self, home: &str) -> Self {
        self.home = PathBuf::from(home);
        self
    }

    pub(super) fn with_var(mut self, name: &str, value: &str) -> Self {
        self.vars.insert(name.to_owned(), value.to_owned());
        self
    }

    pub(super) fn with_file(mut self, path: &str, contents: &str) -> Self {
        self.files.insert(PathBuf::from(path), contents.to_owned());
        self
    }

    pub(super) fn unreadable(mut self, path: &str) -> Self {
        self.unreadable.insert(PathBuf::from(path));
        self
    }
}

impl SapEnvironment for FakeEnvironment {
    fn var(&self, name: &str) -> Option<String> {
        self.vars.get(name).cloned()
    }

    fn file_exists(&self, path: &Path) -> bool {
        self.files.contains_key(path) || self.unreadable.contains(path)
    }

    fn read_file(&self, path: &Path) -> Result<String, io::Error> {
        self.files.get(path).map_or_else(
            || Err(io::Error::from(io::ErrorKind::PermissionDenied)),
            |contents| Ok(contents.clone()),
        )
    }

    fn home_dir(&self) -> PathBuf {
        self.home.clone()
    }
}

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

pub(super) fn at(epoch_seconds: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(epoch_seconds, 0).unwrap()
}

pub(super) struct FixedClock {
    pub(super) now: DateTime<Utc>,
}

impl SapClock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.now
    }
}

pub(super) struct ScriptedTransport {
    pub(super) responses: Mutex<VecDeque<Result<String, TokenTransportError>>>,
    pub(super) requests: Mutex<Vec<String>>,
}

impl ScriptedTransport {
    pub(super) fn request_count(&self) -> usize {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    pub(super) fn request_urls(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl SapTokenTransport for ScriptedTransport {
    fn post(&self, request: &TokenRequest) -> Result<String, TokenTransportError> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.url.clone());
        self.responses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap()
    }
}

pub(super) struct MapConfigurations {
    pub(super) entries: BTreeMap<String, Json>,
    pub(super) failures: BTreeSet<String>,
}

impl MapConfigurations {
    pub(super) fn new(entries: &[(&str, &str)]) -> Self {
        Self {
            entries: entries
                .iter()
                .map(|(id, body)| ((*id).to_owned(), json(body)))
                .collect(),
            failures: BTreeSet::new(),
        }
    }

    pub(super) fn failing(mut self, id: &str) -> Self {
        self.failures.insert(id.to_owned());
        self
    }
}

impl DeploymentConfigurations for MapConfigurations {
    fn configuration(&self, configuration_id: &str) -> Result<Json, ConfigurationLookupError> {
        if self.failures.contains(configuration_id) {
            return Err(ConfigurationLookupError::Missing(
                configuration_id.to_owned(),
            ));
        }
        self.entries
            .get(configuration_id)
            .cloned()
            .ok_or_else(|| ConfigurationLookupError::Missing(configuration_id.to_owned()))
    }
}

pub(super) fn base_credentials() -> Credentials {
    Credentials {
        client_id: Some("client-id".to_owned()),
        client_secret: None,
        auth_url: Some("https://auth.example.com/oauth/token".to_owned()),
        base_url: Some("https://api.example.com/v2".to_owned()),
        cert_file_path: None,
        key_file_path: None,
        cert_str: None,
        key_str: None,
        resource_group: "default".to_owned(),
    }
}
