use crate::credential_contracts::errors::{CredError, CredResult};
use crate::credentials::{CredentialError, McpClient};
use crate::server_record::MCPServer;
use crate::static_credential::validate_static_credential;

use super::{static_auth_type, static_header_slots, stdio_transport};

// The check runs on the request the upstream will actually receive, not on the configured auth value, so a header the transport fills in still counts.
pub fn prepare_mcp_client(server: &MCPServer, client: &McpClient) -> CredResult<()> {
    let Some(auth_type) = static_auth_type(server.fields.auth_type) else {
        return Ok(());
    };
    if stdio_transport(server) {
        return Ok(());
    }
    let request = client.prepare_request_auth().map_err(prepare_failure)?;
    let slots = static_header_slots(server);
    validate_static_credential(
        auth_type,
        &request.headers,
        server.fields.upstream_token_header.as_deref(),
        &slots,
    )
}

fn prepare_failure(error: CredentialError) -> CredError {
    let message = format!("could not prepare the upstream request: {error}");
    CredError::Misconfigured(message)
}
