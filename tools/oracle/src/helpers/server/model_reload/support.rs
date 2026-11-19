use std::collections::BTreeSet;

use async_trait::async_trait;
use betterlite_db::auxiliary_repository::repository::{PrismaClient, TableError};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::resource_repository::model::ModelRepository;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_redis::config_sync::publish::redis_config_change_publisher;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces};
use betterlite_server::model_reload::cache::{
    PublishedModel, ReconcileOutcome, ReloadFailure, ServingRouter,
};
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

// The case's fake llm_router: the published rows clear_cache reads and edits, the four strategy registries, and the planted reload outcome.
pub(super) struct FakeRouter {
    pub(super) models: Vec<PublishedModel>,
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
            models: models
                .iter()
                .map(|entry| PublishedModel {
                    model_name: entry.model_name.clone(),
                    litellm_params: entry.litellm_params.clone(),
                    model_info: entry.model_info.clone(),
                })
                .collect(),
            deleted: Vec::new(),
            strategies: Strategies::new(strategy_names),
            reloads: 0,
            reload_result,
        }
    }

    pub(super) fn remaining(&self) -> Vec<String> {
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
        _repository: &ModelRepository<'_>,
    ) -> Result<ReconcileOutcome, ReloadFailure> {
        self.reloads = self.reloads.saturating_add(1);
        self.reload_result.clone()
    }
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
