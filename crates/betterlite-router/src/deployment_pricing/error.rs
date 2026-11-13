use betterlite_models::model_registration::error::RegistrationError;

use super::deployment::DeploymentReadError;

#[derive(Debug, thiserror::Error)]
pub enum DeploymentPricingError<Fetch> {
    #[error(transparent)]
    Read(#[from] DeploymentReadError),
    #[error(transparent)]
    Registration(#[from] RegistrationError<Fetch>),
}
