use super::convert::{
    self, CallToolWire, GetPromptWire, McpErrorKind, NativeMcpErrorPayload, session_payload,
};
use super::session::NoProgress;
use betterlite_config::tls::TlsPolicy;
use betterlite_core::api::mcp_client_resources::operations::McpResourceOperations as _;
use betterlite_core::api::mcp_client_resources::settings::ResourceSettings;
use betterlite_http::mcp_sigv4::McpSigV4Auth;
use betterlite_mcp::client::sdk_runtime::handler::ServerRequestHandlers;
use betterlite_mcp::client::sdk_runtime::runtime::SdkRuntime;
use betterlite_mcp::client::sdk_transport::error::SdkError;
use betterlite_mcp::client::sessions::client::{SessionClient, SessionSettings};
use betterlite_mcp::client::sessions::error::SessionError;
use betterlite_mcp::client::sessions::negotiate::{Callbacks, ProtocolVersion};
use betterlite_mcp::client::sessions::transport::{StdioConfig, TransportType};
use betterlite_mcp::client::tools::calling;
use betterlite_mcp::client::tools::listing;
use betterlite_mcp::credentials::{
    AuthValue, McpAuthType, McpClient as CredentialClient, hash_discovery_auth,
};
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use url::Url;
// The text every client operation answers Python with, or the payload its failure becomes
pub(crate) type TextResult = Result<String, NativeMcpErrorPayload>;
// A poisoned lock still carries the token and instructions the client last saw, so read it through
fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}
// One native client owns this state; every Python call builds a fresh session client over it (B4744)
pub(crate) struct ClientState {
    pub(crate) server_url: String,
    pub(crate) transport_type: TransportType,
    pub(crate) stdio: Option<StdioConfig>,
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) npm_cache_dir: String,
    pub(crate) tls: TlsPolicy,
    pub(crate) protocol_version: ProtocolVersion,
    pub(crate) auth_type: McpAuthType,
    pub(crate) auth_header_name: Option<String>,
    pub(crate) extra_headers: BTreeMap<String, String>,
    pub(crate) timeout: Duration,
    pub(crate) listing_timeout: Duration,
    pub(crate) auth_value: Mutex<Option<AuthValue>>,
    pub(crate) aws: Option<Arc<McpSigV4Auth>>,
    pub(crate) runtime: Arc<SdkRuntime>,
    pub(crate) instructions: Mutex<Option<String>>,
    pub(crate) tracker: TaskTracker,
}
impl ClientState {
    pub(crate) fn session_client(&self) -> SessionClient<SdkRuntime> {
        let tracker = TaskTracker::new();
        let cancelled = CancellationToken::new();
        self.session_client_with(tracker, cancelled)
    }

    pub(crate) fn session_client_with(
        &self,
        tracker: TaskTracker,
        caller_cancelled: CancellationToken,
    ) -> SessionClient<SdkRuntime> {
        SessionClient {
            runtime: Arc::clone(&self.runtime),
            auth: self.credential_client(),
            settings: SessionSettings {
                transport_type: self.transport_type,
                stdio: self.stdio.clone(),
                environment: self.environment.clone(),
                npm_cache_dir: self.npm_cache_dir.clone(),
                tls: self.tls.clone(),
                protocol_version: self.protocol_version,
                client_version: env!("CARGO_PKG_VERSION").to_owned(),
                callbacks: Callbacks::default(),
                handlers: ServerRequestHandlers::default(),
            },
            tracker,
            caller_cancelled,
            last_initialize_instructions: None,
        }
    }

    // B4741 owns the header slot, the Basic encoding and the empty-value rule; this only carries the last value in
    fn credential_client(&self) -> CredentialClient {
        let mut client = CredentialClient::new(
            self.server_url.clone(),
            self.auth_type,
            None,
            self.auth_header_name.clone(),
            self.extra_headers.clone(),
            self.timeout,
            None,
            None,
        );
        client = client.with_shared_auth(None, self.aws.clone());
        let stored = Option::clone(&locked(&self.auth_value));
        if let Some(value) = stored {
            client.update_auth_value(value);
        }
        client
    }

    pub(crate) fn remember_instructions(&self, client: &SessionClient<SdkRuntime>) {
        let mut guard = locked(&self.instructions);
        let carried = client.last_initialize_instructions.clone();
        *guard = carried;
    }

    pub(crate) fn set_auth_value(&self, value: AuthValue) {
        let mut guard = locked(&self.auth_value);
        let stored = Some(value);
        *guard = stored;
    }

    pub(crate) fn discovery_auth_fingerprint(&self) -> TextResult {
        let request = self
            .credential_client()
            .prepare_request_auth()
            .map_err(|error| {
                NativeMcpErrorPayload::plain(McpErrorKind::RuntimeError, error.to_string())
            })?;
        hash_discovery_auth(&request).map_err(|error| {
            NativeMcpErrorPayload::plain(McpErrorKind::RuntimeError, error.to_string())
        })
    }

    fn resource_settings(&self) -> ResourceSettings {
        ResourceSettings {
            listing_timeout: self.listing_timeout,
        }
    }

    fn answer<T>(
        &self,
        client: &SessionClient<SdkRuntime>,
        outcome: Result<T, SessionError<SdkError>>,
    ) -> TextResult
    where
        T: Serialize,
    {
        self.remember_instructions(client);
        let value = outcome.map_err(|error| session_payload(&error))?;
        convert::encode(&value)
    }

    pub(crate) async fn list_tools(&self, raise_on_error: bool) -> TextResult {
        let mut client = self.session_client();
        let outcome = listing::list_tools(&mut client, self.listing_timeout, raise_on_error).await;
        self.answer(&client, outcome)
    }

    pub(crate) async fn call_tool(&self, wire: &CallToolWire) -> TextResult {
        let request = convert::call_request(wire);
        let options = convert::call_options(wire);
        let mut client = self.session_client();
        let outcome = calling::call_tool(&mut client, &request, None::<&NoProgress>, options).await;
        self.answer(&client, outcome)
    }

    pub(crate) async fn list_prompts(&self, raise_on_error: bool) -> TextResult {
        let settings = self.resource_settings();
        let mut client = self.session_client();
        let outcome = client.list_prompts(&settings, raise_on_error).await;
        self.answer(&client, outcome)
    }

    pub(crate) async fn list_resources(&self, raise_on_error: bool) -> TextResult {
        let settings = self.resource_settings();
        let mut client = self.session_client();
        let outcome = client.list_resources(&settings, raise_on_error).await;
        self.answer(&client, outcome)
    }

    pub(crate) async fn list_resource_templates(&self, raise_on_error: bool) -> TextResult {
        let settings = self.resource_settings();
        let mut client = self.session_client();
        let outcome = client
            .list_resource_templates(&settings, raise_on_error)
            .await;
        self.answer(&client, outcome)
    }

    pub(crate) async fn get_prompt(&self, wire: GetPromptWire) -> TextResult {
        let params = convert::prompt_params(wire);
        let mut client = self.session_client();
        let outcome = client.get_prompt(params).await;
        self.answer(&client, outcome)
    }

    pub(crate) async fn read_resource(&self, url: &str) -> TextResult {
        let endpoint = Url::parse(url).map_err(NativeMcpErrorPayload::from)?;
        let mut client = self.session_client();
        let outcome = client.read_resource(endpoint).await;
        self.answer(&client, outcome)
    }
}
