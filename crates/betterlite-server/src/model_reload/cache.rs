use std::collections::BTreeSet;

use async_trait::async_trait;
use betterlite_db::resource_repository::model::ModelRepository;
use betterlite_models::decisions::Json;
use betterlite_router::catalog_updates::catalog::CatalogHandle;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use tokio::sync::Mutex;

const AUTO_ROUTER_PREFIX: &str = "auto_router/";

// ReconcileOutcome: what a reload observed while it held the reconcile gate; both fields are None when no reload ran.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReconcileOutcome {
    pub still_desired: Option<BTreeSet<String>>,
    pub live_after: Option<BTreeSet<String>>,
}

// _add_deployment_locked's failure, kept as the message clear_cache logs.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("{message}")]
pub struct ReloadFailure {
    pub message: String,
}

// This pod's llm_router as clear_cache sees it: the published catalog, the deployment and strategy removals, and the DB reload that rebuilds them.
#[async_trait]
pub trait ServingRouter: Send {
    fn catalog(&self) -> &CatalogHandle;

    fn delete_deployment(&mut self, model_id: &str);

    fn drop_strategy_routers(&mut self, model_names: &BTreeSet<String>);

    // proxy_config._add_deployment_locked: the caller already holds the reconcile gate, so this must not take it again.
    async fn reload(
        &mut self,
        repository: &ModelRepository<'_>,
    ) -> Result<ReconcileOutcome, ReloadFailure>;
}

// clear_cache: rebuild this pod's DB models under the reconcile gate, wiping only the auto-router deployments and their strategy entries.
#[must_use]
pub async fn clear_cache<R>(
    gate: &Mutex<()>,
    router: Option<&mut R>,
    repository: Option<&ModelRepository<'_>>,
) -> ReconcileOutcome
where
    R: ServingRouter + ?Sized,
{
    let connected = matches!(repository, Some(found) if found.prisma_client().is_ok());
    let (Some(router), true, Some(repository)) = (router, connected, repository) else {
        tracing::debug!("llm_router or prisma_client is None, skipping cache clear");
        return ReconcileOutcome::default();
    };
    let _serialized = gate.lock().await;
    match reconcile_locked(router, repository).await {
        Ok(outcome) => outcome,
        Err(failure) => {
            tracing::error!("Failed to clear cache and reload models. Due to error - {failure}");
            ReconcileOutcome::default()
        }
    }
}

async fn reconcile_locked<R>(
    router: &mut R,
    repository: &ModelRepository<'_>,
) -> Result<ReconcileOutcome, ReloadFailure>
where
    R: ServingRouter + ?Sized,
{
    tracing::debug!("Clearing only DB models, preserving config models");
    let published = router.catalog().load_full();
    let mut config_models = 0_usize;
    let mut db_models = 0_usize;
    let mut db_router_names = BTreeSet::new();
    for model in &published.models {
        if !is_db_model(model) {
            config_models = config_models.saturating_add(1);
            continue;
        }
        db_models = db_models.saturating_add(1);
        if is_auto_router(model) {
            db_router_names.insert(model.model_name.clone());
            if let Some(id) = model.id() {
                router.delete_deployment(id);
            }
        }
    }
    router.drop_strategy_routers(&db_router_names);
    let outcome = router.reload(repository).await?;
    tracing::debug!("Reconciled {db_models} DB models, preserved {config_models} config models");
    Ok(outcome)
}

fn is_db_model(model: &DeploymentSpec) -> bool {
    match model.model_info.get("db_model") {
        Some(Json::Bool(db_model)) => *db_model,
        _ => false,
    }
}

fn is_auto_router(model: &DeploymentSpec) -> bool {
    match model.litellm_params.get("model") {
        Some(Json::Text(model)) => model.starts_with(AUTO_ROUTER_PREFIX),
        _ => false,
    }
}
