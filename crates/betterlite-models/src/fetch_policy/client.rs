use std::error::Error;
use std::future::Future;
use std::time::Duration;

use crate::fetch_policy::response::FetchResponse;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FetchErrorKind {
    InvalidUrl,
    Transport,
}

pub trait CatalogClientError: Error + Send + Sync + 'static {
    fn kind(&self) -> FetchErrorKind;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogRequest {
    pub url: String,
    pub timeout: Duration,
}

pub trait CatalogClient: Send + Sync {
    type Error: CatalogClientError;

    fn get(&self, request: &CatalogRequest) -> Result<FetchResponse, Self::Error>;

    fn get_async(
        &self,
        request: &CatalogRequest,
    ) -> impl Future<Output = Result<FetchResponse, Self::Error>> + Send;
}
