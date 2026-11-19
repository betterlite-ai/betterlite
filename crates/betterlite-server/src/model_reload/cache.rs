use std::collections::BTreeSet;

use async_trait::async_trait;
use betterlite_db::resource_repository::model::ModelRepository;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
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

// One row this pod's router serves: LiteLLM's model_list entry, which clear_cache classifies as config or DB.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublishedModel {
    pub model_name: String,
    pub litellm_params: JsonObject,
    pub model_info: JsonObject,
}

impl PublishedModel {
    #[must_use]
    pub fn id(&self) -> Option<&str> {
        match self.model_info.get("id") {
            Some(Json::Text(id)) => Some(id),
            _ => None,
        }
    }
}

// The catalog snapshot's deployment as clear_cache reads it: the three fields it inspects, without the passthrough extra.
impl From<&DeploymentSpec> for PublishedModel {
    fn from(spec: &DeploymentSpec) -> Self {
        Self {
            model_name: spec.model_name.clone(),
            litellm_params: spec.litellm_params.clone(),
            model_info: spec.model_info.clone(),
        }
    }
}

// This pod's llm_router as clear_cache sees it: the published rows, the deployment and strategy removals, and the DB reload that rebuilds them.
#[async_trait]
pub trait ServingRouter: Send {
    // Owned rows, so the router's own catalog type never crosses this seam and any serving router can implement it.
    fn published_models(&self) -> Vec<PublishedModel>;

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
    let (Some(active_router), true, Some(active_repository)) = (router, connected, repository)
    else {
        tracing::debug!("llm_router or prisma_client is None, skipping cache clear");
        return ReconcileOutcome::default();
    };
    let _serialized = gate.lock().await;
    match reconcile_locked(active_router, active_repository).await {
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
    let published = router.published_models();
    let mut config_models = 0_usize;
    let mut db_models = 0_usize;
    let mut db_router_names = BTreeSet::new();
    for model in &published {
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

fn is_db_model(model: &PublishedModel) -> bool {
    match model.model_info.get("db_model") {
        Some(Json::Bool(db_model)) => *db_model,
        _ => false,
    }
}

fn is_auto_router(model: &PublishedModel) -> bool {
    match model.litellm_params.get("model") {
        Some(Json::Text(name)) => name.starts_with(AUTO_ROUTER_PREFIX),
        _ => false,
    }
}
