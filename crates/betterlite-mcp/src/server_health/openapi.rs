use std::future::Future;
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use betterlite_db::managed_resource_rows::mcp::McpStatus;
use betterlite_http::proxy::transport_error::TransportError;
use tokio::time::timeout;

use super::health::Health;
use super::ports::{AllowedServers, Clock, EnvVarCheck, ProbeClient, SessionProbe, SpecLoader};
use super::service::HealthService;
use crate::openapi_tools::error::OpenApiError;

pub const OPENAPI_PROBE_TTL: Duration = Duration::from_secs(30);
pub const MAX_SPEC_BYTES: usize = 10 * 1024 * 1024;

const NO_PROTOCOL_PROBE: &str = "OpenAPI servers have no protocol-level health probe";
const CANCELLED: &str = "OpenAPI specification check was cancelled";
const UNLOADABLE: &str = "OpenAPI specification could not be loaded";

#[derive(Debug, Default)]
pub struct OpenApiHealthProbe {
    checked_at: Option<Duration>,
    result: Option<Health>,
}

// multiple_inherent_impl leaves service.rs the type's one inherent impl, so the spec check lives on this trait
pub trait SpecHealthChecks {
    #[must_use]
    fn spec_health(&self, spec_path: &str) -> impl Future<Output = Health> + Send;
}

impl<P, S, E, A, C, L> SpecHealthChecks for HealthService<P, S, E, A, C, L>
where
    P: ProbeClient,
    S: SessionProbe,
    E: EnvVarCheck,
    A: AllowedServers,
    C: Clock,
    L: SpecLoader,
{
    async fn spec_health(&self, spec_path: &str) -> Health {
        let probe = {
            let mut slots = self
                .spec_probes
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            Arc::clone(slots.entry(spec_path.to_owned()).or_default())
        };
        let mut entry = probe.lock().await;
        let now = self.deps.clock.now_monotonic();
        if let Some(cached) = &entry.result
            && entry
                .checked_at
                .is_some_and(|checked_at| now.saturating_sub(checked_at) < OPENAPI_PROBE_TTL)
        {
            return cached.clone();
        }
        let check = openapi_spec_health(
            &self.deps.specs,
            spec_path,
            self.settings.health_check_timeout,
        );
        let Some((status, error)) = self.deps.cancel.run_until_cancelled(check).await else {
            return Health {
                status: McpStatus::Unknown,
                error: Some(CANCELLED.to_owned()),
                checked_at: self.deps.clock.now_utc(),
            };
        };
        let health = Health {
            status,
            error,
            checked_at: self.deps.clock.now_utc(),
        };
        entry.result = Some(health.clone());
        entry.checked_at = Some(self.deps.clock.now_monotonic());
        health
    }
}

#[must_use]
pub async fn openapi_spec_health<L>(
    loader: &L,
    spec_path: &str,
    limit: Duration,
) -> (McpStatus, Option<String>)
where
    L: SpecLoader,
{
    if !spec_path.starts_with("http://") && !spec_path.starts_with("https://") {
        return (McpStatus::Unknown, Some(NO_PROTOCOL_PROBE.to_owned()));
    }
    match timeout(limit, loader.load(spec_path, MAX_SPEC_BYTES)).await {
        Err(_elapsed) => (
            McpStatus::Unhealthy,
            Some(format!(
                "OpenAPI specification check timed out after {:?} seconds",
                limit.as_secs_f64()
            )),
        ),
        Ok(Ok(())) => (McpStatus::Healthy, None),
        Ok(Err(error)) => spec_failure(&error),
    }
}

// LiteLLM names the Python exception class each loader failure stands for
fn spec_failure(error: &OpenApiError) -> (McpStatus, Option<String>) {
    match error {
        OpenApiError::SpecStatus { status, .. } => (
            McpStatus::Unhealthy,
            Some(format!(
                "OpenAPI specification request failed (HTTP {status})"
            )),
        ),
        OpenApiError::SpecBody {
            source: source @ TransportError::BodyTooLarge { .. },
            ..
        } => (
            McpStatus::Unknown,
            Some(format!("OpenAPI specification probe refused: {source}")),
        ),
        OpenApiError::Json { .. } => (
            McpStatus::Unhealthy,
            Some(format!("{UNLOADABLE} (JSONDecodeError)")),
        ),
        OpenApiError::SpecNotFound { .. } => (
            McpStatus::Unhealthy,
            Some(format!("{UNLOADABLE} (FileNotFoundError)")),
        ),
        OpenApiError::SpecRead { .. } => (
            McpStatus::Unhealthy,
            Some(format!("{UNLOADABLE} (OSError)")),
        ),
        OpenApiError::SpecFetch { .. } => (
            McpStatus::Unhealthy,
            Some(format!("{UNLOADABLE} (RequestError)")),
        ),
        OpenApiError::SpecBody { .. } => (
            McpStatus::Unhealthy,
            Some(format!("{UNLOADABLE} (ReadError)")),
        ),
    }
}
