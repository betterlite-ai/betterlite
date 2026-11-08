use std::future::Future;
use std::time::Duration;

use crate::fetch_policy::response::FetchResponse;

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum FetchClientError {
    #[error("the catalog URL is not valid: {detail}")]
    InvalidUrl { detail: String },
    #[error("the catalog request failed: {detail}")]
    Transport { detail: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogRequest {
    pub url: String,
    pub timeout: Duration,
}

pub trait CatalogClient: Send + Sync {
    fn get(&self, request: &CatalogRequest) -> Result<FetchResponse, FetchClientError>;

    fn get_async(
        &self,
        request: &CatalogRequest,
    ) -> impl Future<Output = Result<FetchResponse, FetchClientError>> + Send;
}
