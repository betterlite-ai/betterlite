use crate::deployment_pricing::deployment::DeploymentReadError;

// Each way resolving or projecting one deployment's credentials fails.
#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    #[error("the catalog index points at row {index}, which is not in the model list")]
    InvalidRow { index: usize },
    #[error(transparent)]
    Deployment(#[from] DeploymentReadError),
    #[error("the deployment credential field {field} is not {expected}")]
    FieldType {
        field: String,
        expected: &'static str,
    },
}
