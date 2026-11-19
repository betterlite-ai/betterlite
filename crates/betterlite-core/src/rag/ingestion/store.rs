use std::error::Error as StdError;

use async_trait::async_trait;

pub type StoreResult = (Option<String>, Option<String>);

pub struct StoreRequest<'request> {
    pub file_content: Option<&'request [u8]>,
    pub filename: Option<&'request str>,
    pub content_type: Option<&'request str>,
    pub chunks: &'request [String],
    pub embeddings: Option<&'request [Vec<f64>]>,
    pub existing_file_id: Option<&'request str>,
}

#[async_trait]
pub trait RagStore: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    async fn store(&self, request: StoreRequest<'_>) -> Result<StoreResult, Self::Error>;
}
