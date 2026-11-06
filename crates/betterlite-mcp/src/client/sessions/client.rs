use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::tls::TlsPolicy;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::debug;

use crate::credentials;

use super::environment::safe_stdio_env;
use super::error::{SessionConfigError, SessionError};
use super::factory::{AuthSource, HttpClientConfig, RedirectGuard, SseRequest, TlsSettings};
use super::negotiate::{Callbacks, ProtocolVersion};
use super::runtime::McpRuntime;
use super::transport::{StdioConfig, StdioServerParameters, TransportContexts, TransportType};

pub struct SessionSettings {
    pub transport_type: TransportType,
    pub stdio: Option<StdioConfig>,
    pub environment: BTreeMap<String, String>,
    pub npm_cache_dir: String,
    pub tls: TlsPolicy,
    pub protocol_version: ProtocolVersion,
    pub client_version: String,
    pub callbacks: Callbacks,
}

pub struct SessionClient<Runtime>
where
    Runtime: McpRuntime,
{
    pub runtime: Arc<Runtime>,
    pub auth: credentials::McpClient,
    pub settings: SessionSettings,
    pub tracker: TaskTracker,
    pub caller_cancelled: CancellationToken,
    pub last_initialize_instructions: Option<String>,
}

impl<Runtime: McpRuntime> SessionClient<Runtime> {
    pub fn create_transport_context(
        &self,
    ) -> Result<TransportContexts<Runtime>, SessionError<Runtime::Error>> {
        match self.settings.transport_type {
            TransportType::Stdio => {
                let config = self
                    .settings
                    .stdio
                    .as_ref()
                    .ok_or(SessionConfigError::MissingStdioConfig)?;
                let parameters = StdioServerParameters {
                    command: config.command.clone(),
                    args: config.args.clone(),
                    env: safe_stdio_env(
                        config.env.as_ref(),
                        &self.settings.environment,
                        &self.settings.npm_cache_dir,
                    ),
                };
                Ok(TransportContexts {
                    transport: self.runtime.stdio_client(parameters),
                    http_client: None,
                })
            }
            TransportType::Sse => Ok(self.sse_context()),
            TransportType::Http => self.http_context(),
        }
    }

    fn sse_context(&self) -> TransportContexts<Runtime> {
        let headers = self.auth.auth_headers();
        let client = self.create_http_client_config(&headers, self.auth.timeout, false);
        TransportContexts {
            transport: self.runtime.sse_client(SseRequest {
                url: self.auth.server_url.clone(),
                timeout: self.auth.timeout,
                headers,
                client,
            }),
            http_client: None,
        }
    }

    fn http_context(&self) -> Result<TransportContexts<Runtime>, SessionError<Runtime::Error>> {
        debug!("MCP client uses the streamable HTTP transport");
        let headers = self.auth.auth_headers();
        let config = self.create_http_client_config(&headers, self.auth.timeout, false);
        let client = Arc::new(
            self.runtime
                .http_client(config)
                .map_err(|error| SessionError::Runtime(Arc::new(error)))?,
        );
        let transport = self
            .runtime
            .streamable_http_client(self.auth.server_url.clone(), Arc::clone(&client));
        Ok(TransportContexts {
            transport,
            http_client: Some(client),
        })
    }

    #[must_use]
    pub fn create_http_client_config(
        &self,
        headers: &BTreeMap<String, String>,
        timeout: Duration,
        explicit_auth: bool,
    ) -> HttpClientConfig {
        let auth = if explicit_auth {
            AuthSource::Explicit
        } else if self.auth.resolved_auth.is_some() {
            AuthSource::Resolved
        } else if self.auth.aws_auth.is_some() {
            AuthSource::Aws
        } else {
            AuthSource::None
        };
        HttpClientConfig {
            headers: headers.clone(),
            timeout,
            tls: TlsSettings::from_policy(&self.settings.tls),
            auth,
            follow_redirects: true,
            redirect_guard: RedirectGuard::new(
                &self.auth.server_url,
                self.auth.credential_slot.as_deref(),
            ),
            capture_error_responses: true,
        }
    }
}
