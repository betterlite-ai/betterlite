use std::fmt::{self, Debug, Formatter};
use std::future::Future;

use crate::aws_settings::sources::Credentials;
use crate::bedrock_embedding_io::transport::TransferError;

pub const BEDROCK_SERVICE: &str = "bedrock";

// the signed headers carry a provider credential, so Debug names them without their values
#[derive(Clone, PartialEq, Eq)]
pub struct ControlPlaneRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Debug for ControlPlaneRequest {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = self.headers.iter().map(|(name, _)| name.as_str()).collect();
        f.debug_struct("ControlPlaneRequest")
            .field("method", &self.method)
            .field("url", &self.url)
            .field("header_names", &names)
            .field("body_length", &self.body.len())
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlPlaneReply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

pub trait ControlPlaneTransport: Send + Sync {
    fn send(
        &self,
        request: ControlPlaneRequest,
    ) -> impl Future<Output = Result<ControlPlaneReply, TransferError>> + Send;
}

// the boto3.client("bedrock", ...) constructor this slice stands in for
#[derive(Clone, Copy)]
pub struct ClientConfig<'source> {
    pub service_name: &'source str,
    pub region: &'source str,
    pub credentials: &'source Credentials,
    pub signature_version: &'source str,
}

impl Debug for ClientConfig<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientConfig")
            .field("service_name", &self.service_name)
            .field("region", &self.region)
            .field("signature_version", &self.signature_version)
            .finish()
    }
}

pub trait ControlPlaneFactory: Sync {
    type Client: ControlPlaneTransport;

    fn client(&self, config: ClientConfig<'_>) -> Self::Client;
}

#[must_use]
pub fn header<'headers>(
    headers: &'headers [(String, String)],
    name: &str,
) -> Option<&'headers str> {
    headers
        .iter()
        .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}
