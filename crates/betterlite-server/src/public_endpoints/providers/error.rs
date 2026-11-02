#[derive(Debug, thiserror::Error)]
pub enum ProvidersError {
    #[error("provider catalog fragment is {size} bytes, over the {limit} byte limit")]
    FragmentTooLarge { size: usize, limit: usize },
    #[error("provider catalog fragment JSON error: {0}")]
    FragmentJson(#[source] serde_json::Error),
    #[error("endpoint support file is {size} bytes, over the {limit} byte limit")]
    EndpointsTooLarge { size: usize, limit: usize },
    #[error("endpoint support file JSON error: {0}")]
    EndpointsJson(#[source] serde_json::Error),
}
