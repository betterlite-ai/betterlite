use crate::aws_endpoints::error::AwsEndpointError;

#[derive(Debug, thiserror::Error)]
pub enum MantleInputError {
    #[error(transparent)]
    Region(#[from] AwsEndpointError),
}
