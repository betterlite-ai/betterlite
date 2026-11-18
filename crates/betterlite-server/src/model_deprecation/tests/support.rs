use std::collections::BTreeMap;
use std::io;
use std::str;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use chrono::{DateTime, NaiveDate, Utc};
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use crate::model_deprecation::catalog::{CatalogDeprecation, DeprecationCatalog};
use crate::model_deprecation::clock::UtcClock;
use crate::model_deprecation::types::ModelDeprecationInfo;

pub(crate) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(crate) fn object(entries: Vec<(&str, Json)>) -> JsonObject {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

pub(crate) fn calendar_date(value: &str) -> NaiveDate {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").unwrap()
}

pub(crate) fn moment(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .unwrap()
        .with_timezone(&Utc)
}

pub(crate) fn names(models: &[ModelDeprecationInfo]) -> Vec<&str> {
    models
        .iter()
        .map(|model| model.model_name.as_str())
        .collect()
}

pub(crate) fn spec(name: &str, model: &str, model_info: JsonObject) -> DeploymentSpec {
    DeploymentSpec::new(
        name.to_owned(),
        object(vec![("model", text(model))]),
        model_info,
        JsonObject::new(),
        String::new,
    )
    .unwrap()
}

pub(crate) struct TestCatalog {
    entries: BTreeMap<String, (NaiveDate, Option<String>)>,
}

impl TestCatalog {
    pub(crate) fn new(entries: Vec<(&str, &str, Option<&str>)>) -> Self {
        Self {
            entries: entries
                .into_iter()
                .map(|(key, deprecation_date, provider)| {
                    (
                        key.to_owned(),
                        (calendar_date(deprecation_date), provider.map(str::to_owned)),
                    )
                })
                .collect(),
        }
    }
}

impl DeprecationCatalog for TestCatalog {
    fn deprecation(&self, model_key: &str) -> Option<CatalogDeprecation> {
        let (deprecation_date, litellm_provider) = self.entries.get(model_key)?;
        Some(CatalogDeprecation {
            deprecation_date: *deprecation_date,
            litellm_provider: litellm_provider.clone(),
        })
    }
}

pub(crate) struct FixedClock(pub(crate) DateTime<Utc>);

impl UtcClock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

#[derive(Clone, Default)]
pub(crate) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub(crate) fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        str::from_utf8(&bytes)
            .expect("the captured log lines are UTF-8")
            .to_owned()
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for Captured {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

pub(crate) fn capture_debug_logs() -> (Captured, DefaultGuard) {
    let buffer = Captured::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}
