use std::error::Error;
use std::time::Duration;

pub const QUERY_TIMEOUT: Duration = Duration::from_secs(30);
pub const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QueryRequest {
    pub url: String,
    pub params: Vec<(&'static str, String)>,
    pub timeout: Duration,
    pub max_response_bytes: usize,
}

pub trait Transport: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn get(
        &self,
        request: QueryRequest,
    ) -> impl Future<Output = Result<Vec<u8>, Self::Error>> + Send;
}
