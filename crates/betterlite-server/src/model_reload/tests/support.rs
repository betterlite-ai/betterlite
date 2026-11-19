use std::collections::BTreeSet;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_db::auxiliary_repository::repository::{PrismaClient, TableError};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::resource_repository::model::ModelRepository;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_redis::config_sync::publish::redis_config_change_publisher;
use betterlite_router::catalog_updates::catalog::{CatalogHandle, DeploymentCatalog};
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces};
use secrecy::SecretString;
use tokio::sync::Mutex as AsyncMutex;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt;
use tracing_subscriber::fmt::MakeWriter;

use crate::model_reload::cache::{PublishedModel, ReconcileOutcome, ReloadFailure, ServingRouter};
use crate::model_reload::verdict::WrittenModel;

#[derive(Clone, Default)]
pub(crate) struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    pub(crate) fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        str::from_utf8(&bytes)
            .expect("the captured log lines are UTF-8")
            .to_owned()
    }
}

impl io::Write for LogBuffer {
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

impl MakeWriter<'_> for LogBuffer {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

pub(crate) fn capture_logs(level: Level) -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(level)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}

pub(crate) fn value(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture is JSON")
}

pub(crate) fn object(text: &str) -> JsonObject {
    serde_json::from_str(text).expect("the fixture is a JSON object")
}

pub(crate) fn written_model(id: &str, model_info: Json) -> WrittenModel {
    WrittenModel {
        id: id.to_owned(),
        model_info,
    }
}

pub(crate) fn ids(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

pub(crate) fn deployment(
    model_name: &str,
    id: &str,
    model: &str,
    db_model: bool,
) -> DeploymentSpec {
    let mut model_info = JsonObject::new();
    model_info.insert("id".to_owned(), Json::Text(id.to_owned()));
    model_info.insert("db_model".to_owned(), Json::Bool(db_model));
    let mut litellm_params = JsonObject::new();
    litellm_params.insert("model".to_owned(), Json::Text(model.to_owned()));
    DeploymentSpec {
        model_name: model_name.to_owned(),
        litellm_params,
        model_info,
        extra: JsonObject::new(),
    }
}

pub(crate) fn deployment_without_id(model_name: &str) -> DeploymentSpec {
    DeploymentSpec {
        model_name: model_name.to_owned(),
        litellm_params: object(r#"{"model":"openai/gpt-4o"}"#),
        model_info: object(r#"{"db_model":true}"#),
        extra: JsonObject::new(),
    }
}

pub(crate) fn catalog(models: &[DeploymentSpec]) -> CatalogHandle {
    let mut catalog = DeploymentCatalog::default();
    for model in models {
        catalog.push(model.clone());
    }
    CatalogHandle::from_pointee(catalog)
}

#[derive(Default)]
pub(crate) struct Strategies {
    pub(crate) auto: BTreeSet<String>,
    pub(crate) complexity: BTreeSet<String>,
    pub(crate) adaptive: BTreeSet<String>,
    pub(crate) quality: BTreeSet<String>,
}

pub(crate) struct FakeRouter {
    pub(crate) models: Vec<PublishedModel>,
    pub(crate) deleted: Vec<String>,
    pub(crate) strategies: Strategies,
    pub(crate) reloads: usize,
    pub(crate) database_connected: bool,
    pub(crate) reload_result: Result<ReconcileOutcome, ReloadFailure>,
    pub(crate) gate: Option<Arc<AsyncMutex<()>>>,
    pub(crate) gate_held: bool,
}

impl FakeRouter {
    pub(crate) fn new(models: &[DeploymentSpec], strategy_names: &[&str]) -> Self {
        let mut strategies = Strategies::default();
        for name in strategy_names {
            let owned = String::from(*name);
            strategies.auto.insert(owned.clone());
            strategies.complexity.insert(owned.clone());
            strategies.adaptive.insert(owned.clone());
            strategies.quality.insert(owned);
        }
        Self {
            models: models.iter().map(PublishedModel::from).collect(),
            deleted: Vec::new(),
            strategies,
            reloads: 0,
            database_connected: false,
            reload_result: Ok(ReconcileOutcome::default()),
            gate: None,
            gate_held: false,
        }
    }

    pub(crate) fn remaining(&self) -> Vec<String> {
        self.models
            .iter()
            .filter_map(|model| model.id().map(str::to_owned))
            .collect()
    }
}

#[async_trait]
impl ServingRouter for FakeRouter {
    fn published_models(&self) -> Vec<PublishedModel> {
        self.models.clone()
    }

    fn delete_deployment(&mut self, model_id: &str) {
        self.deleted.push(model_id.to_owned());
        self.models.retain(|model| model.id() != Some(model_id));
    }

    fn drop_strategy_routers(&mut self, model_names: &BTreeSet<String>) {
        self.strategies
            .auto
            .retain(|name| !model_names.contains(name));
        self.strategies
            .complexity
            .retain(|name| !model_names.contains(name));
        self.strategies
            .adaptive
            .retain(|name| !model_names.contains(name));
        self.strategies
            .quality
            .retain(|name| !model_names.contains(name));
    }

    async fn reload(
        &mut self,
        repository: &ModelRepository<'_>,
    ) -> Result<ReconcileOutcome, ReloadFailure> {
        self.reloads = self.reloads.saturating_add(1);
        self.database_connected = repository.prisma_client().is_ok();
        self.gate_held = self
            .gate
            .as_ref()
            .is_some_and(|gate| gate.try_lock().is_err());
        match &self.reload_result {
            Ok(outcome) => Ok(outcome.clone()),
            Err(failure) => Err(failure.clone()),
        }
    }
}

pub(crate) struct Database {
    store: PostgresStore,
    availability: Availability,
}

impl Database {
    pub(crate) fn new() -> Self {
        Self {
            store: PostgresStore { pool: None },
            availability: Availability::default(),
        }
    }

    pub(crate) fn repository(&self) -> ModelRepository<'_> {
        let client = PrismaClient {
            writer: &self.store,
            reader: &self.store,
            availability: &self.availability,
        };
        repository(Some(client))
    }
}

pub(crate) fn repository_without_client() -> ModelRepository<'static> {
    repository(None)
}

fn repository(client: Option<PrismaClient<'_>>) -> ModelRepository<'_> {
    ModelRepository::new(
        client,
        redis_config_change_publisher::<TableError>(None),
        None,
        cipher(),
    )
}

fn cipher() -> DbCipher {
    DbCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from("model-reload-tests-key".to_owned())),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}
