use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_provider::api::vector_store_contracts::config::Params;
use betterlite_provider::api::vector_store_contracts::error::VectorStoreEmbeddingError;
use secrecy::SecretString;

use super::error::ValkeyError;
use super::query::ValkeyKnnQuery;

pub type ValkeySearchParams = BTreeMap<String, Vec<u8>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValkeyDocument {
    pub id: String,
    pub fields: BTreeMap<String, String>,
}

#[async_trait]
pub trait ValkeySearchPort: Send + Sync {
    async fn search(
        &self,
        index: &str,
        query: &ValkeyKnnQuery,
        params: &ValkeySearchParams,
    ) -> Result<Vec<ValkeyDocument>, ValkeyError>;
}

#[async_trait]
pub trait ValkeyEmbedder: Send + Sync {
    async fn embed(
        &self,
        model: &str,
        text: &str,
        configuration: &Params,
    ) -> Result<Vec<Vec<f32>>, VectorStoreEmbeddingError>;
}

#[async_trait]
pub trait ValkeyConnector: Send + Sync {
    async fn connect(
        &self,
        url: &SecretString,
        timeouts: SocketTimeouts,
    ) -> Result<Box<dyn ValkeySearchPort>, ValkeyError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SocketTimeouts {
    pub connect: Duration,
    pub read: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValkeyTimeout {
    None,
    Uniform(Duration),
    ConnectRead {
        connect: Option<Duration>,
        read: Option<Duration>,
    },
}

pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
pub const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(30);

#[must_use]
pub fn socket_timeouts(timeout: ValkeyTimeout) -> SocketTimeouts {
    match timeout {
        ValkeyTimeout::None => SocketTimeouts {
            connect: DEFAULT_CONNECT_TIMEOUT,
            read: DEFAULT_READ_TIMEOUT,
        },
        ValkeyTimeout::Uniform(read) => SocketTimeouts {
            connect: read.min(DEFAULT_CONNECT_TIMEOUT),
            read,
        },
        ValkeyTimeout::ConnectRead { connect, read } => SocketTimeouts {
            connect: connect.unwrap_or(DEFAULT_CONNECT_TIMEOUT),
            read: read.unwrap_or(DEFAULT_READ_TIMEOUT),
        },
    }
}
