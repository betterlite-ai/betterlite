use std::future::Future;
use std::io;

use super::auth::RequestHeader;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitLabResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

// An implementation bounds the body to max_body_bytes and applies its own request timeout.
pub trait GitLabTransport: Send + Sync {
    fn get(
        &self,
        url: &str,
        headers: &[RequestHeader],
        max_body_bytes: usize,
    ) -> impl Future<Output = Result<GitLabResponse, io::Error>> + Send;
}
