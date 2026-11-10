use async_trait::async_trait;
use http::Method;
use thiserror::Error;
use uuid::Uuid;

use super::error::QdrantSemanticError;
use crate::arguments::json::Object;

#[derive(Debug, Error)]
pub enum QdrantEmbedError {
    #[error("{message}")]
    Failed { message: String },
}

#[async_trait]
pub trait QdrantEmbedder: Send + Sync {
    async fn embed(
        &self,
        prompt: &str,
        metadata: Option<&Object>,
    ) -> Result<Vec<f64>, QdrantEmbedError>;
}

#[derive(Clone, Debug)]
pub struct QdrantRequest {
    pub method: Method,
    pub path: String,
    pub query: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
}

#[derive(Clone, Debug)]
pub struct QdrantReply {
    pub status: u16,
    pub body: Vec<u8>,
}

#[async_trait]
pub trait QdrantTransport: Send + Sync {
    async fn send(&self, request: QdrantRequest) -> Result<QdrantReply, QdrantSemanticError>;
}

pub trait UuidSource: Send + Sync {
    fn next_id(&self) -> Uuid;
}

#[derive(Debug, Clone, Copy)]
pub struct SystemUuid;

impl UuidSource for SystemUuid {
    fn next_id(&self) -> Uuid {
        Uuid::new_v4()
    }
}
