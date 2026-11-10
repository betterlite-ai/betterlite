use std::io;

use crate::azure_client_secret::ClientSecretError;
use crate::azure_client_secret::authority;
use crate::azure_client_secret::transport as client_secret;

use super::transport::{HttpRequest, Transport};

// the wrapper never passes a tenant id, so the credential keeps the SDK's work-and-school-account tenant
pub const ORGANIZATIONS_TENANT: &str = "organizations";

pub fn token_endpoint(
    transport: &dyn Transport,
    authority_host: &str,
) -> Result<String, ClientSecretError> {
    let bridged = Bridged { transport };
    authority::discover_authority(&bridged, authority_host, ORGANIZATIONS_TENANT, true)
        .map(|authority| authority.token_endpoint)
}

struct Bridged<'transport> {
    transport: &'transport dyn Transport,
}

impl client_secret::Transport for Bridged<'_> {
    fn send(&self, request: &client_secret::HttpRequest) -> io::Result<client_secret::HttpReply> {
        let bridged = HttpRequest {
            method: request.method,
            url: request.url.clone(),
            query: request.query.clone(),
            headers: request.headers.clone(),
            form: request.form.clone(),
        };
        self.transport.send(&bridged)
    }
}
