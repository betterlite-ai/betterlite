use betterlite_models::decisions::Json;
use betterlite_models::model_registration::{
    CostMapFetcher, ModelInfoLookup, ModelRegistry, RegistrationEnvironment,
};

use super::PricingInputs;
use super::deployment::Deployment;
use super::error::DeploymentPricingError;
use super::register::register_deployment_pricing;

pub async fn replay_deployment_pricing<Lookup, Fetcher>(
    registry: &mut ModelRegistry,
    environment: &RegistrationEnvironment<'_, Lookup, Fetcher>,
    deployments: &[Json],
    inputs: &PricingInputs,
) -> Result<(), DeploymentPricingError<Fetcher::Error>>
where
    Lookup: ModelInfoLookup,
    Fetcher: CostMapFetcher,
{
    for entry in deployments {
        let registered = match Deployment::from_entry(entry) {
            Ok(deployment) => {
                register_deployment_pricing(
                    registry,
                    deployment.litellm_params,
                    deployment.model_info,
                    environment,
                    inputs,
                )
                .await
            }
            Err(unreadable) => Err(DeploymentPricingError::from(unreadable)),
        };
        match registered {
            Ok(()) => {}
            Err(DeploymentPricingError::Registration(failure)) => {
                return Err(DeploymentPricingError::Registration(failure));
            }
            Err(DeploymentPricingError::Read(unreadable)) => tracing::error!(
                error = %unreadable,
                "Router: could not rebuild cost-map entry for a deployment during a price data reload"
            ),
        }
    }
    Ok(())
}
