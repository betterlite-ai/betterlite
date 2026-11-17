use rmcp::ServiceError;
use rmcp::model::{
    ClientNotification, ClientRequest, Implementation, InitializeRequest as SdkInitializeRequest,
    InitializeRequestParams, InitializeResult, InitializedNotification, ProtocolVersion,
    ServerPeerInfo, ServerResult,
};

use crate::client::sdk_transport::error::SdkError;
use crate::client::sessions::error::McpFailure;
use crate::client::sessions::negotiate::{
    ClientSession, InitializeOutcome, InitializeRequest as SessionInitializeRequest,
};

use super::super::convert::{
    DEFAULT_CLIENT_NAME, DEFAULT_CLIENT_VERSION, capabilities_wire, client_capabilities,
    protocol_failure as sdk_protocol_failure, protocol_version, unexpected_response,
};
use super::{SdkSession, Silent, exchange_with_progress, request_failure};

// The SDK's handshake lists, from HANDSHAKE_PROTOCOL_VERSIONS in mcp_types.version
const HANDSHAKE_VERSIONS: [ProtocolVersion; 4] = [
    ProtocolVersion::V_2024_11_05,
    ProtocolVersion::V_2025_03_26,
    ProtocolVersion::V_2025_06_18,
    ProtocolVersion::V_2025_11_25,
];

pub struct SdkInitializeResult {
    result: InitializeResult,
}

impl InitializeOutcome for SdkInitializeResult {
    fn protocol_version(&self) -> &str {
        self.result.protocol_version.as_str()
    }

    fn instructions(&self) -> Option<&str> {
        self.result.instructions.as_deref()
    }
}

async fn send_initialize_request(
    session: &SdkSession,
    params: InitializeRequestParams,
) -> Result<InitializeResult, SdkError> {
    let request = SdkInitializeRequest::new(params);
    let response =
        exchange_with_progress(session, ClientRequest::InitializeRequest(request), &Silent).await?;
    let ServerResult::InitializeResult(result) = response else {
        return Err(unexpected_response(&response));
    };
    Ok(result)
}

impl ClientSession for SdkSession {
    type Error = SdkError;
    type InitializeResult = SdkInitializeResult;

    // The SDK's own handshake: its latest version with an initialize step, and the capabilities its callbacks answer
    async fn initialize(&mut self) -> Result<SdkInitializeResult, SdkError> {
        let params = InitializeRequestParams::new(
            self.capabilities.clone(),
            Implementation::new(DEFAULT_CLIENT_NAME, DEFAULT_CLIENT_VERSION),
        )
        .with_protocol_version(ProtocolVersion::LATEST_WITH_INITIALIZE);
        let result = send_initialize_request(self, params).await?;
        // The SDK rejects a version outside its handshake before it adopts the result or sends notifications/initialized
        if !HANDSHAKE_VERSIONS.contains(&result.protocol_version) {
            return Err(SdkError::Service(ServiceError::UnexpectedResponse));
        }
        let outcome = SdkInitializeResult { result };
        self.adopt(&outcome);
        self.send_initialized().await?;
        Ok(outcome)
    }

    async fn send_initialize(
        &mut self,
        request: SessionInitializeRequest,
    ) -> Result<SdkInitializeResult, SdkError> {
        let version = protocol_version(&request.protocol_version)?;
        let capabilities = client_capabilities(&request.capabilities);
        let params = InitializeRequestParams::new(
            capabilities,
            Implementation::new(request.client_info.name, request.client_info.version),
        )
        .with_protocol_version(version);
        let result = send_initialize_request(self, params).await?;
        Ok(SdkInitializeResult { result })
    }

    // The SDK adopts after the version check, which B4744 keeps, so the server's capabilities and instructions land here
    fn adopt(&mut self, result: &SdkInitializeResult) {
        if let Some(protocol_version) = self.protocol_version.as_ref() {
            protocol_version.set(result.protocol_version());
        }
        self.server_capabilities = Some(capabilities_wire(&result.result.capabilities));
        self.peer
            .set_peer_info(ServerPeerInfo::from(result.result.clone()));
    }

    async fn send_initialized(&mut self) -> Result<(), SdkError> {
        self.peer
            .send_notification(ClientNotification::InitializedNotification(
                InitializedNotification::default(),
            ))
            .await
            .map_err(|error| request_failure(self, "notifications/initialized", error))
    }

    fn protocol_failure(error: &SdkError) -> Option<McpFailure> {
        sdk_protocol_failure(error)
    }
}
