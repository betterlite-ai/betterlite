use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use chrono::{DateTime, Utc};
use serde_json::Number;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::router_predicates::JsonMap;
use crate::usage_routing::window::{Counts, UsageWindows, WindowError};

// The group and deployment a success event names, with the metadata and model_info maps a callback reads.
pub(super) fn success_params(group: &str, id: Json) -> JsonMap {
    let metadata =
        JsonObject::from_iter([("model_group".to_owned(), Json::Text(group.to_owned()))]);
    let model_info = JsonObject::from_iter([("id".to_owned(), id)]);
    JsonMap::from_iter([
        ("metadata".to_owned(), Json::Object(metadata)),
        ("model_info".to_owned(), Json::Object(model_info)),
    ])
}

pub(super) fn counts(entries: &[(&str, i64)]) -> Counts {
    entries
        .iter()
        .map(|(id, count)| ((*id).to_owned(), *count))
        .collect()
}

pub(super) fn deployment(
    id: &str,
    top: &[(&str, Json)],
    params: &[(&str, Json)],
    info: &[(&str, Json)],
) -> DeploymentSpec {
    let mut litellm_params =
        JsonObject::from_iter([("model".to_owned(), Json::Text("openai/probe".to_owned()))]);
    for (key, value) in params {
        litellm_params.insert((*key).to_owned(), value.clone());
    }
    let mut model_info = JsonObject::from_iter([("id".to_owned(), Json::Text(id.to_owned()))]);
    for (key, value) in info {
        model_info.insert((*key).to_owned(), value.clone());
    }
    let extra = JsonObject::from_iter(
        top.iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone())),
    );
    DeploymentSpec::new(
        "gpt-4o".to_owned(),
        litellm_params,
        model_info,
        extra,
        String::new,
    )
    .unwrap()
}

pub(super) fn plain(id: &str) -> DeploymentSpec {
    deployment(id, &[], &[], &[])
}

pub(super) fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

pub(super) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(super) fn instant(unix_millis: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(unix_millis).expect("the fixture clock is in range")
}

// The window store the success callbacks write and get_available_deployments reads, with its failures injected.
#[derive(Debug, Default)]
pub(super) struct LocalWindows {
    counts: Mutex<BTreeMap<String, Counts>>,
    ttls: Mutex<BTreeMap<String, i64>>,
    read_failures: AtomicBool,
    write_failures: AtomicBool,
}

impl LocalWindows {
    pub(super) fn seed(&self, key: &str, entries: &[(&str, i64)]) {
        self.counts
            .lock()
            .unwrap()
            .insert(key.to_owned(), counts(entries));
    }

    pub(super) fn window(&self, key: &str) -> Option<Counts> {
        self.counts.lock().unwrap().get(key).cloned()
    }

    pub(super) fn ttl(&self, key: &str) -> Option<i64> {
        self.ttls.lock().unwrap().get(key).copied()
    }

    pub(super) fn fail_reads(&self, flag: bool) {
        self.read_failures.store(flag, Ordering::Relaxed);
    }

    pub(super) fn fail_writes(&self, flag: bool) {
        self.write_failures.store(flag, Ordering::Relaxed);
    }
}

impl UsageWindows for LocalWindows {
    fn read(&self, key: &str) -> Result<Option<Counts>, WindowError> {
        if self.read_failures.load(Ordering::Relaxed) {
            return Err(WindowError::Unavailable);
        }
        Ok(self.counts.lock().unwrap().get(key).cloned())
    }

    fn write(&self, key: &str, counts: &Counts, ttl_seconds: i64) -> Result<(), WindowError> {
        if self.write_failures.load(Ordering::Relaxed) {
            return Err(WindowError::Unavailable);
        }
        self.counts
            .lock()
            .unwrap()
            .insert(key.to_owned(), counts.clone());
        self.ttls
            .lock()
            .unwrap()
            .insert(key.to_owned(), ttl_seconds);
        Ok(())
    }
}
