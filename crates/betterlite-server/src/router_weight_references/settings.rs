use std::collections::BTreeSet;

use betterlite_db::resource_repository::model::ModelRepository;
use betterlite_models::decisions::Json;
use betterlite_router::catalog_updates::catalog::DeploymentCatalog;

use super::error::RouterWeightError;
use super::reference::{RouterWeightDeployment, stored_deployments, validate_weight_reference};
use super::weights::decode_router_weights;

pub async fn validate_settings_weights(
    router_settings: Option<&Json>,
    team_id: Option<&str>,
    models: Option<&ModelRepository<'_>>,
    configured: Option<&DeploymentCatalog>,
) -> Result<(), RouterWeightError> {
    let Some(settings) = router_settings else {
        return Ok(());
    };
    let Some(weights) = decode_router_weights(settings)? else {
        return Ok(());
    };
    let deployment_ids = weights
        .values()
        .flat_map(|group| group.keys())
        .map(|deployment_id| deployment_id.0.as_str())
        .collect::<BTreeSet<_>>();
    if deployment_ids.is_empty() {
        return Ok(());
    }
    let Some(repository) = models else {
        return Err(RouterWeightError::DatabaseUnavailable);
    };
    let stored = stored_deployments(repository, &deployment_ids).await?;
    for (model_group, group) in &weights {
        for deployment_id in group.keys().map(|identifier| identifier.0.as_str()) {
            let stored_deployment = stored.get(deployment_id);
            let configured_deployment = if stored_deployment.is_some() {
                None
            } else {
                configured_reference(configured, deployment_id)?
            };
            validate_weight_reference(
                &model_group.0,
                deployment_id,
                team_id,
                stored_deployment,
                configured_deployment.as_ref(),
            )?;
        }
    }
    Ok(())
}

fn configured_reference(
    configured: Option<&DeploymentCatalog>,
    deployment_id: &str,
) -> Result<Option<RouterWeightDeployment>, RouterWeightError> {
    let Some(catalog) = configured else {
        return Ok(None);
    };
    let Some(index) = catalog.id_index.get(deployment_id) else {
        return Ok(None);
    };
    let Some(spec) = catalog.models.get(*index) else {
        return Ok(None);
    };
    Ok(Some(RouterWeightDeployment::from_configured(spec)?))
}
