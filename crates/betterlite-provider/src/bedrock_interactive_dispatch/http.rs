use std::future::Future;
use std::pin::Pin;

use betterlite_http::proxy::transport_error::TransportError;

// What every interactive leaf built once its own signing answered; the route sends it unchanged
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeafRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, Option<String>)>,
    pub body: Vec<u8>,
}

// The reply a transport hands back; chunks carry the pieces a streaming collector is fed
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeafReply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub chunks: Vec<Vec<u8>>,
}

// The one seam every interactive leaf sends through, so a fixture captures the request unchanged
pub trait LeafTransport: Send + Sync {
    fn send(
        &self,
        request: LeafRequest,
    ) -> Pin<Box<dyn Future<Output = Result<LeafReply, TransportError>> + Send + '_>>;
}
