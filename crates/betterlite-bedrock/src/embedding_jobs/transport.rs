use std::fmt::{self, Debug, Formatter};
use std::future::Future;

use crate::bedrock_embedding_io::transport::TransferError;

#[derive(Clone, PartialEq, Eq)]
pub struct StatusWireRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
}

// the signed headers carry a provider credential, so Debug names them without their values
impl Debug for StatusWireRequest {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = self.headers.iter().map(|(name, _)| name.as_str()).collect();
        formatter
            .debug_struct("StatusWireRequest")
            .field("url", &self.url)
            .field("header_names", &names)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusReply {
    pub status: u16,
    pub body: Vec<u8>,
}

pub trait StatusTransport: Send + Sync {
    fn get(
        &self,
        request: StatusWireRequest,
    ) -> impl Future<Output = Result<StatusReply, TransferError>> + Send;
}
