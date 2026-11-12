use crate::model_registration::error::RegistrationError;

#[derive(Debug, thiserror::Error)]
pub enum DeploymentIdError {
    #[error("model_info.id in {bucket} is neither a string nor a number")]
    NotScalar { bucket: &'static str },
}

#[derive(Debug, thiserror::Error)]
pub enum RequestPricingError<Fetch> {
    #[error(transparent)]
    DeploymentId(#[from] DeploymentIdError),
    #[error(transparent)]
    Registration(#[from] RegistrationError<Fetch>),
}
