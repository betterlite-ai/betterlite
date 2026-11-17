use rmcp::model::{
    ClientNotification, Implementation, InitializeRequest as SdkInitializeRequest,
    InitializeRequestParams, InitializedNotification, ProtocolVersion, ServerPeerInfo,
    ServerResult,
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
use super::SdkSession;

pub struct SdkInitializeResult {
    result: rmcp::model::InitializeResult,
}

impl SdkInitializeResult {
    fn from_result(result: rmcp::model::InitializeResult) -> Self {
        Self { result }
    }
}

impl InitializeOutcome for SdkInitializeResult {
    fn protocol_version(&self) -> &str {
        self.result.protocol_version.as_str()
    }

    fn instructions(&self) -> Option<&str> {
        self.result.instructions.as_deref()
    }
}

impl SdkSession {
    async fn send_initialize_request(
        &self,
        params: InitializeRequestParams,
    ) -> Result<rmcp::model::InitializeResult, SdkError> {
        let request = SdkInitializeRequest::new(params);
        match self
            .exchange(rmcp::model::ClientRequest::InitializeRequest(request))
            .await?
        {
            ServerResult::InitializeResult(result) => Ok(result),
            other => Err(unexpected_response(&other)),
        }
    }
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
        let outcome = SdkInitializeResult::from_result(self.send_initialize_request(params).await?);
        self.adopt(&outcome);
        self.send_initialized().await?;
        Ok(outcome)
    }

    async fn send_initialize(
        &mut self,
        request: SessionInitializeRequest,
    ) -> Result<SdkInitializeResult, SdkError> {
        let version = protocol_version(&request.protocol_version)?;
        let capabilities = client_capabilities(
            request.capabilities.sampling,
            request.capabilities.elicitation,
        );
        let params = InitializeRequestParams::new(
            capabilities,
            Implementation::new(request.client_info.name, request.client_info.version),
        )
        .with_protocol_version(version);
        let result = self.send_initialize_request(params).await?;
        Ok(SdkInitializeResult::from_result(result))
    }

    // The SDK adopts after the version check, which B4744 keeps, so the server's capabilities and instructions land here
    fn adopt(&mut self, result: &SdkInitializeResult) {
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
            .map_err(|error| self.request_failure("notifications/initialized", error))
    }

    fn protocol_failure(error: &SdkError) -> Option<McpFailure> {
        sdk_protocol_failure(error)
    }
}
