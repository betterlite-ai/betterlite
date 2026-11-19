use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_auth::auth_model_lists::credential::{CredentialEntry, CredentialSnapshot};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::catalog_updates::catalog::DeploymentCatalog;
use crate::deployment_identity::spec::DeploymentSpec;

pub(crate) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(crate) fn pairs(values: &[(&str, Json)]) -> JsonObject {
    values
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

// One model-list row as _create_deployment stores it: an id, the params it declares, and its model_info.
pub(crate) fn spec(
    model_name: &str,
    id: &str,
    params: &[(&str, Json)],
    info: &[(&str, Json)],
) -> DeploymentSpec {
    let litellm_params = pairs(params);
    let mut model_info = pairs(info);
    model_info.insert("id".to_owned(), text(id));
    DeploymentSpec::new(
        model_name.to_owned(),
        litellm_params,
        model_info,
        JsonObject::new(),
        String::new,
    )
    .unwrap()
}

pub(crate) fn row(model_name: &str, id: &str, model: &str) -> DeploymentSpec {
    spec(model_name, id, &[("model", text(model))], &[])
}

// set_model_list: every row registered with its wildcards, then pushed under the catalog's indices.
pub(crate) fn catalog_of(rows: Vec<DeploymentSpec>) -> DeploymentCatalog {
    let mut catalog = DeploymentCatalog::default();
    for row in rows {
        catalog.index_patterns(&row).unwrap();
        catalog.push(row);
    }
    catalog
}

pub(crate) fn snapshot(entries: &[(&str, JsonObject)]) -> CredentialSnapshot {
    CredentialSnapshot::new(
        entries
            .iter()
            .map(|(name, values)| CredentialEntry {
                credential_name: (*name).to_owned(),
                credential_values: values.clone(),
            })
            .collect(),
    )
}

#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<String>>>);

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Logs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() != Level::WARN {
            return;
        }
        let mut message = Message::default();
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message.0);
    }
}

// The WARN messages production logs while the call runs.
pub(crate) fn captured<T, F>(run: F) -> (T, Vec<String>)
where
    F: FnOnce() -> T,
{
    let logs = Logs::default();
    let subscriber = tracing_subscriber::registry().with(logs.clone());
    let value = with_default(subscriber, run);
    let warnings = logs
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    (value, warnings)
}
