use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::model_registration::{CostMapFetcher, ModelInfoLookup};

use super::admission::normalize;
use super::catalog::{CatalogHandle, DeploymentCatalog};
use super::entries::{ModelEntry, deployment_inputs};
use super::error::CatalogError;
use super::ptu_warning::warn_unattributed_ptu;
use super::skip::skip_or_raise;
use crate::deployment_identity::construct::{Construction, ConstructionPolicy, create_deployment};
use crate::deployment_identity::error::{DeploymentError, InvalidDeployment};
use crate::deployment_identity::spec::DeploymentSpec;
use crate::deployment_pricing::register::register_deployment_pricing;

// What a catalog update asks of the tasks that own the provider check, the strategies, the budgets and the caches.
pub trait CatalogEffects: Sync {
    // _add_deployment's provider check and strategy initialisation
    fn admit(&self, spec: &DeploymentSpec) -> Result<(), InvalidDeployment>;

    // _sync_deployment_budget_config
    fn registered(&self, spec: &DeploymentSpec);

    // the budget unregister, the pre-routing strategy release and the discovered model info eviction of a removed deployment
    fn released(&self, spec: &DeploymentSpec);

    // _deployment_participates_in_adaptive_routing
    fn participates_in_adaptive(&self, spec: &DeploymentSpec) -> bool;

    // _finalize_adaptive_router_if_configured over the catalog about to be published
    fn finalize_adaptive(&self, catalog: &DeploymentCatalog);

    // the model group info and access group caches, cleared once a new catalog is published
    fn invalidated(&self);
}

// The only writer of a catalog: it builds the next one beside the published one and swaps it in when the update succeeded.
pub struct CatalogUpdater<'run, Lookup, Fetcher, Policy, Effects> {
    pub catalog: &'run CatalogHandle,
    pub effects: &'run Effects,
    // where the os.environ/ references in a deployment's data sources are read
    pub env_vars: &'run (dyn EnvironmentLookup + Sync),
    pub construction: Construction<'run, Lookup, Fetcher, Policy>,
}

type Failure<Fetcher> = CatalogError<<Fetcher as CostMapFetcher>::Error>;

impl<Lookup, Fetcher, Policy, Effects> CatalogUpdater<'_, Lookup, Fetcher, Policy, Effects>
where
    Lookup: ModelInfoLookup,
    Fetcher: CostMapFetcher,
    Policy: ConstructionPolicy,
    Effects: CatalogEffects,
{
    // the edits, the provider check, the strategies and the wildcard routers of _add_deployment
    pub(super) fn admit(
        &self,
        working: &mut DeploymentCatalog,
        spec: &mut DeploymentSpec,
    ) -> Result<(), Failure<Fetcher>> {
        normalize(spec, self.env_vars);
        self.effects.admit(spec)?;
        working.index_patterns(spec)?;
        Ok(())
    }

    pub(super) fn commit(&self, working: DeploymentCatalog) {
        self.catalog.store(Arc::new(working));
        self.effects.invalidated();
    }

    // add_deployment on a working copy: the deployment as added, or none when the id is already there
    async fn add_into(
        &mut self,
        working: &mut DeploymentCatalog,
        mut spec: DeploymentSpec,
    ) -> Result<Option<DeploymentSpec>, Failure<Fetcher>> {
        if spec
            .id()
            .is_some_and(|id| working.id_index.contains_key(id))
        {
            return Ok(None);
        }
        self.construction
            .policy
            .warn_credential_mismatch(&spec.model_name, &spec.litellm_params);
        self.admit(working, &mut spec)?;
        register_deployment_pricing(
            self.construction.registry,
            &spec.litellm_params,
            &spec.model_info,
            self.construction.environment,
            self.construction.pricing,
        )
        .await
        .map_err(DeploymentError::from)?;
        working.model_names.insert(spec.model_name.clone());
        self.effects.registered(&spec);
        working.push(spec.clone());
        Ok(Some(spec))
    }

    // Router.add_deployment: none when the id is already in the catalog.
    pub async fn add_deployment(
        &mut self,
        spec: DeploymentSpec,
    ) -> Result<Option<DeploymentSpec>, Failure<Fetcher>> {
        let mut working = (*self.catalog.load_full()).clone();
        let Some(added) = self.add_into(&mut working, spec).await? else {
            return Ok(None);
        };
        self.commit(working);
        Ok(Some(added))
    }

    // Router.upsert_deployment: none when nothing changed, or when an ignored failure left the previous deployment serving.
    pub async fn upsert_deployment(
        &mut self,
        spec: DeploymentSpec,
    ) -> Result<Option<DeploymentSpec>, Failure<Fetcher>> {
        let id = spec.id().unwrap_or_default().to_owned();
        let snapshot = self.catalog.load_full();
        let previous = snapshot
            .id_index
            .get(&id)
            .and_then(|index| snapshot.models.get(*index))
            .cloned();
        if previous.as_ref().is_some_and(|old| {
            old.model_name == spec.model_name
                && old.litellm_params == spec.litellm_params
                && old.model_info == spec.model_info
        }) {
            return Ok(None);
        }
        let mut working = (*snapshot).clone();
        if let Some(old) = &previous {
            if let Some(index) = working.id_index.get(&id).copied() {
                working.remove(&id, index);
            }
            self.effects.released(old);
        }
        match self.add_into(&mut working, spec.clone()).await {
            Ok(added) => {
                self.finish_upsert(working, &spec, previous.as_ref());
                Ok(added)
            }
            Err(error) => {
                self.drop_failed_upsert(&snapshot, &spec, previous, error)
                    .await
            }
        }
    }

    // A failed upsert is returned in raise mode; otherwise it is logged and the previous deployment is put back.
    async fn drop_failed_upsert(
        &mut self,
        snapshot: &DeploymentCatalog,
        spec: &DeploymentSpec,
        previous: Option<DeploymentSpec>,
        error: Failure<Fetcher>,
    ) -> Result<Option<DeploymentSpec>, Failure<Fetcher>> {
        if !self.construction.ignore_invalid_deployments {
            return Err(error);
        }
        let id = spec.id().unwrap_or_default();
        tracing::warn!(
            "Error upserting deployment {} (id={id}): {error}. Dropping it and continuing with other deployments.",
            spec.model_name,
        );
        if let Some(old) = previous {
            self.restore(snapshot, id, old).await;
        }
        Ok(None)
    }

    // The adaptive router is finalized over the new catalog when the new or the replaced deployment took part in it.
    fn finish_upsert(
        &self,
        working: DeploymentCatalog,
        spec: &DeploymentSpec,
        previous: Option<&DeploymentSpec>,
    ) {
        if self.effects.participates_in_adaptive(spec)
            || previous.is_some_and(|old| self.effects.participates_in_adaptive(old))
        {
            self.effects.finalize_adaptive(&working);
        }
        self.commit(working);
    }

    // _restore_deployment_after_failed_upsert: the previous deployment goes back at the end of the list.
    async fn restore(&mut self, snapshot: &DeploymentCatalog, id: &str, previous: DeploymentSpec) {
        let mut working = snapshot.clone();
        if let Some(index) = working.id_index.get(id).copied() {
            working.remove(id, index);
        }
        let name = previous.model_name.clone();
        match self.add_into(&mut working, previous).await {
            Ok(_) => {
                self.commit(working);
                tracing::info!(
                    "Restored deployment {name} (id={id}); it keeps serving its previous configuration."
                );
            }
            Err(error) => tracing::warn!(
                "Could not restore previously served deployment {name} (id={id}) after the failed upsert: {error}"
            ),
        }
    }

    // Router.delete_deployment: the removed deployment, or none when the id is unknown.
    #[must_use]
    pub fn delete_deployment(&self, id: &str) -> Option<DeploymentSpec> {
        let snapshot = self.catalog.load_full();
        let index = *snapshot.id_index.get(id)?;
        let mut working = (*snapshot).clone();
        let removed = working.remove(id, index)?;
        self.commit(working);
        self.effects.released(&removed);
        Some(removed)
    }

    // Router.set_model_list: the whole list is built apart and replaces the published catalog only when it was built.
    pub async fn set_model_list(&mut self, entries: &[ModelEntry]) -> Result<(), Failure<Fetcher>> {
        warn_unattributed_ptu(entries, self.construction.pricing.ptu_cost_attribution);
        let mut working = DeploymentCatalog::default();
        for input in deployment_inputs(entries) {
            let created = create_deployment(&mut self.construction, &input).await?;
            let unresolved = match created.deployment {
                Some(spec) => self.add_created(&mut working, spec)?,
                None => created.provider_unresolved,
            };
            if unresolved {
                working.provider_unresolved.push(input);
            }
        }
        working.model_names = entries
            .iter()
            .map(|entry| entry.model_name.clone())
            .collect();
        self.effects.finalize_adaptive(&working);
        self.commit(working);
        Ok(())
    }

    // The add step of _create_deployment: whether the provider could not be resolved, so the input waits for a retry.
    fn add_created(
        &self,
        working: &mut DeploymentCatalog,
        mut spec: DeploymentSpec,
    ) -> Result<bool, Failure<Fetcher>> {
        match self.admit(working, &mut spec) {
            Ok(()) => {
                working.push(spec);
                Ok(false)
            }
            Err(error) => skip_or_raise(self.construction.ignore_invalid_deployments, error),
        }
    }
}
