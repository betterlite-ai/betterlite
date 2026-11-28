use std::future::Future;

use betterlite_db::managed_resource_rows::mcp::{McpServerTable, McpStatus};
use betterlite_db::mcp_stored_contracts::MCPAuth;
use secrecy::ExposeSecret as _;
use tracing::warn;

use super::health::Health;
use super::health_row;
use super::instructions::ServerInstructions as _;
use super::openapi::SpecHealthChecks as _;
use super::ports::{
    AllowedServers, Clock, EnvVarCheck, ProbeClient, SessionError, SessionProbe, SpecLoader,
};
use super::reachability::ServerReachability as _;
use super::service::HealthService;
use crate::registry::lookup::ServerLookups as _;
use crate::server_record::{MCPServer, ServerFields};

const CANCELLED: &str = "Health check was cancelled";

// multiple_inherent_impl leaves service.rs the type's one inherent impl, so the check steps live on this trait
pub trait ServerHealthChecks {
    #[must_use]
    fn check_server(&self, server_id: &str) -> impl Future<Output = McpServerTable> + Send;
    fn probe_health(&self, server: &MCPServer) -> impl Future<Output = Health> + Send;
    fn uses_reachability(&self, server: &MCPServer) -> bool;
    fn session_health(
        &self,
        server: &MCPServer,
    ) -> impl Future<Output = (McpStatus, Option<String>)> + Send;
}

impl<P, S, E, A, C, L> ServerHealthChecks for HealthService<P, S, E, A, C, L>
where
    P: ProbeClient,
    S: SessionProbe,
    E: EnvVarCheck,
    A: AllowedServers,
    C: Clock,
    L: SpecLoader,
{
    async fn check_server(&self, server_id: &str) -> McpServerTable {
        let found = {
            let registry = self.deps.registry.read().await;
            registry.server_by_id(server_id, None).cloned()
        };
        let Some(server) = found else {
            warn!("MCP Server {server_id} not found");
            return health_row::not_found_row(server_id, self.deps.clock.now_utc());
        };
        let maybe_spec_path = server
            .fields
            .spec_path
            .as_deref()
            .filter(|path| !path.is_empty());
        let Some(spec_path) = maybe_spec_path else {
            let health = self.probe_health(&server).await;
            let registry = self.deps.registry.read().await;
            return health_row::session_row(&registry, &server, &health);
        };
        let health = self.spec_health(spec_path).await;
        let registry = self.deps.registry.read().await;
        health_row::spec_row(&registry, &server, &health)
    }

    async fn probe_health(&self, server: &MCPServer) -> Health {
        let outcome = if self.uses_reachability(server) {
            self.reachability(server, self.settings.health_check_timeout)
                .await
        } else {
            self.session_health(server).await
        };
        let (status, error) = outcome;
        Health {
            status,
            error,
            checked_at: self.deps.clock.now_utc(),
        }
    }

    fn uses_reachability(&self, server: &MCPServer) -> bool {
        server.requires_per_user_auth()
            || needs_an_unavailable_token(&server.fields)
            || self.deps.env_vars.references_per_user_env_var(server)
    }

    async fn session_health(&self, server: &MCPServer) -> (McpStatus, Option<String>) {
        let timeout = self.settings.health_check_timeout;
        let probe =
            self.deps
                .sessions
                .run_noop(server, server.fields.static_headers.as_ref(), timeout);
        let Some(outcome) = self.deps.cancel.run_until_cancelled(probe).await else {
            return (McpStatus::Unknown, Some(CANCELLED.to_owned()));
        };
        match outcome {
            Ok(instructions) => {
                self.remember_instructions(&server.fields.server_id, instructions.as_deref());
                (McpStatus::Healthy, None)
            }
            Err(SessionError::Timeout) => (
                McpStatus::Unhealthy,
                Some(format!(
                    "Health check timed out after {:?} seconds",
                    timeout.as_secs_f64()
                )),
            ),
            Err(SessionError::Cancelled) => (McpStatus::Unknown, Some(CANCELLED.to_owned())),
            Err(SessionError::Failed(text)) => (McpStatus::Unhealthy, Some(text)),
        }
    }
}

// LiteLLM's third probe-kind rule: a token-needing auth type with no static token cannot open a session
pub(super) fn needs_an_unavailable_token(fields: &ServerFields) -> bool {
    let Some(auth) = fields.auth_type else {
        return false;
    };
    if auth == MCPAuth::None || auth == MCPAuth::AwsSigv4 {
        return false;
    }
    fields
        .authentication_token
        .as_ref()
        .is_none_or(|token| token.0.0.expose_secret().is_empty())
}
