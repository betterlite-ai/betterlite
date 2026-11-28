use std::future::Future;
use std::sync::PoisonError;

use super::check::needs_an_unavailable_token;
use super::ports::{AllowedServers, Clock, EnvVarCheck, ProbeClient, SessionProbe, SpecLoader};
use super::service::HealthService;
use crate::config_loader::ports::InstructionsCache;
use crate::server_record::MCPServer;

// multiple_inherent_impl leaves service.rs the type's one inherent impl, so the prefetch steps live on this trait
pub trait ServerInstructions {
    fn remember_instructions(&self, server_id: &str, instructions: Option<&str>);
    fn ensure_instructions_cached(&self, server: &MCPServer) -> impl Future<Output = ()> + Send;
    fn instructions_are_ready(&self, server: &MCPServer) -> bool;
    fn last_probe_within_cooldown(&self, server_id: &str) -> bool;
    fn record_instructions_probe(&self, server_id: &str);
}

impl<P, S, E, A, C, L> ServerInstructions for HealthService<P, S, E, A, C, L>
where
    P: ProbeClient,
    S: SessionProbe,
    E: EnvVarCheck,
    A: AllowedServers,
    C: Clock,
    L: SpecLoader,
{
    fn remember_instructions(&self, server_id: &str, instructions: Option<&str>) {
        let Some(text) = instructions else {
            return;
        };
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }
        let mut cached = self
            .instructions
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        cached.insert(server_id.to_owned(), trimmed.to_owned());
    }

    async fn ensure_instructions_cached(&self, server: &MCPServer) {
        if self.instructions_are_ready(server) {
            return;
        }
        let server_id = server.fields.server_id.as_str();
        self.record_instructions_probe(server_id);
        let timeout = self.settings.health_check_timeout;
        let outcome = self
            .deps
            .sessions
            .run_noop(server, server.fields.static_headers.as_ref(), timeout)
            .await;
        match outcome {
            Ok(instructions) => self.remember_instructions(server_id, instructions.as_deref()),
            Err(error) => {
                tracing::debug!(
                    "Upstream initialize instructions prefetch failed for {}: {error}",
                    server.fields.name
                );
            }
        }
    }

    // the skip rules of _ensure_upstream_initialize_instructions_cached, in LiteLLM's order
    fn instructions_are_ready(&self, server: &MCPServer) -> bool {
        if server
            .fields
            .spec_path
            .as_deref()
            .is_some_and(|path| !path.is_empty())
        {
            return true;
        }
        if server
            .fields
            .instructions
            .as_deref()
            .is_some_and(|text| !text.trim().is_empty())
        {
            return true;
        }
        let server_id = server.fields.server_id.as_str();
        if self
            .instructions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains_key(server_id)
        {
            return true;
        }
        if server.requires_per_user_auth() {
            return true;
        }
        if self.deps.env_vars.references_per_user_env_var(server) {
            return true;
        }
        if needs_an_unavailable_token(&server.fields) {
            return true;
        }
        self.last_probe_within_cooldown(server_id)
    }

    fn last_probe_within_cooldown(&self, server_id: &str) -> bool {
        let maybe_probed_at = {
            let probed = self
                .instructions_probed_at
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            probed.get(server_id).copied()
        };
        let Some(probed_at) = maybe_probed_at else {
            return false;
        };
        self.deps.clock.now_monotonic().saturating_sub(probed_at)
            < self.settings.health_check_timeout
    }

    fn record_instructions_probe(&self, server_id: &str) {
        let now = self.deps.clock.now_monotonic();
        let mut probed = self
            .instructions_probed_at
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        probed.insert(server_id.to_owned(), now);
    }
}

impl<P, S, E, A, C, L> InstructionsCache for HealthService<P, S, E, A, C, L>
where
    P: ProbeClient,
    S: SessionProbe,
    E: EnvVarCheck,
    A: AllowedServers,
    C: Clock,
    L: SpecLoader,
{
    fn clear(&self) {
        self.instructions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        self.instructions_probed_at
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }
}
