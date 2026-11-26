use async_trait::async_trait;
use betterlite_router::deployment_identity::spec::DeploymentSpec;

use crate::health_cycle::endpoints::HealthCheckResult;
use crate::health_cycle::telemetry::InstrumentationContext;

use super::error::SharedHealthError;

// perform_health_check's arguments as the coordination layer forwards them; the router, catalog and random source stay with the probe.
#[derive(Clone, Copy, Debug)]
pub struct SharedProbeCall<'call> {
    pub models: &'call [DeploymentSpec],
    pub details: bool,
    pub max_concurrency: Option<i64>,
    pub skip_disabled: bool,
    pub instrumentation: Option<&'call InstrumentationContext>,
}

// B4443's perform_health_check, injected: the server hands in the cycle and a test hands in a fake.
#[async_trait]
pub trait SharedHealthProbe: Send + Sync {
    async fn probe(
        &self,
        call: SharedProbeCall<'_>,
    ) -> Result<HealthCheckResult, SharedHealthError>;
}
