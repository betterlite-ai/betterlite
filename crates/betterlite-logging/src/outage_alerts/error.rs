use std::error::Error as StdError;

#[derive(Debug, thiserror::Error)]
pub enum RegionOutageError<DeploymentFailure, ProviderFailure, StoreFailure, SendFailure>
where
    DeploymentFailure: StdError + 'static,
    ProviderFailure: StdError + 'static,
    StoreFailure: StdError + 'static,
    SendFailure: StdError + 'static,
{
    #[error("the deployment lookup failed")]
    Deployments(#[source] DeploymentFailure),
    #[error("the model provider could not be resolved")]
    Providers(#[source] ProviderFailure),
    #[error("the region outage record could not be read")]
    Read(#[source] StoreFailure),
    #[error("the region outage alert could not be sent")]
    Send(#[source] SendFailure),
    #[error("the region outage record could not be stored")]
    Write(#[source] StoreFailure),
}
