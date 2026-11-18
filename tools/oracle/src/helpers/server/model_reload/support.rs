use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_db::auxiliary_repository::repository::{PrismaClient, TableError};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::resource_repository::model::ModelRepository;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_models::model_registration::JsonObject;
use betterlite_redis::config_sync::publish::redis_config_change_publisher;
use betterlite_router::catalog_updates::catalog::{CatalogHandle, DeploymentCatalog};
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces};
use betterlite_server::model_reload::cache::{ReconcileOutcome, ReloadFailure, ServingRouter};
use secrecy::SecretString;

use super::ModelEntry;

#[derive(Default)]
pub(super) struct Strategies {
    pub(super) auto: BTreeSet<String>,
    pub(super) complexity: BTreeSet<String>,
    pub(super) adaptive: BTreeSet<String>,
    pub(super) quality: BTreeSet<String>,
}

impl Strategies {
    fn new(strategy_names: &[String]) -> Self {
        let mut strategies = Self::default();
        for name in strategy_names {
            strategies.auto.insert(name.clone());
            strategies.complexity.insert(name.clone());
            strategies.adaptive.insert(name.clone());
            strategies.quality.insert(name.clone());
        }
        strategies
    }
}

// The case's fake llm_router: the catalog clear_cache reads and edits, the four strategy registries, and the planted reload outcome.
pub(super) struct FakeRouter {
    pub(super) catalog: CatalogHandle,
    pub(super) deleted: Vec<String>,
    pub(super) strategies: Strategies,
    pub(super) reloads: usize,
    reload_result: Result<ReconcileOutcome, ReloadFailure>,
}

impl FakeRouter {
    pub(super) fn new(
        models: &[ModelEntry],
        strategy_names: &[String],
        reload_result: Result<ReconcileOutcome, ReloadFailure>,
    ) -> Self {
        Self {
            catalog: published_catalog(models),
            deleted: Vec::new(),
            strategies: Strategies::new(strategy_names),
            reloads: 0,
            reload_result,
        }
    }
}

#[async_trait]
impl ServingRouter for FakeRouter {
    fn catalog(&self) -> &CatalogHandle {
        &self.catalog
    }

    fn delete_deployment(&mut self, model_id: &str) {
        self.deleted.push(model_id.to_owned());
        let snapshot = self.catalog.load_full();
        if let Some(index) = snapshot.id_index.get(model_id).copied() {
            let mut working = (*snapshot).clone();
            working.remove(model_id, index);
            self.catalog.store(Arc::new(working));
        }
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
        _repository: &ModelRepository<'_>,
    ) -> Result<ReconcileOutcome, ReloadFailure> {
        self.reloads = self.reloads.saturating_add(1);
        self.reload_result.clone()
    }
}

fn published_catalog(models: &[ModelEntry]) -> CatalogHandle {
    let mut catalog = DeploymentCatalog::default();
    for entry in models {
        catalog.push(DeploymentSpec {
            model_name: entry.model_name.clone(),
            litellm_params: entry.litellm_params.clone(),
            model_info: entry.model_info.clone(),
            extra: JsonObject::new(),
        });
    }
    CatalogHandle::from_pointee(catalog)
}

// A database connection clear_cache only checks for: a client with no pool never reaches the network.
pub(super) struct Database {
    store: PostgresStore,
    availability: Availability,
}

impl Database {
    pub(super) fn new() -> Self {
        Self {
            store: PostgresStore { pool: None },
            availability: Availability::default(),
        }
    }

    pub(super) fn repository(&self) -> ModelRepository<'_> {
        ModelRepository::new(
            Some(PrismaClient {
                writer: &self.store,
                reader: &self.store,
                availability: &self.availability,
            }),
            redis_config_change_publisher::<TableError>(None),
            None,
            cipher(),
        )
    }
}

fn cipher() -> DbCipher {
    DbCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from("model-reload-oracle-key".to_owned())),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}
