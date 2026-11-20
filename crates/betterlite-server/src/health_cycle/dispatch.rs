use betterlite_router::deployment_identity::spec::DeploymentSpec;

use crate::health_cycle::concurrency::run_probes;
use crate::health_cycle::endpoints::{HealthCheckResult, separate_results};
use crate::health_cycle::ports::ProviderHealth;
use crate::health_cycle::probe::PreparedProbe;
use crate::health_cycle::telemetry::{InstrumentationContext, dispatch_summary};

// What _perform_health_check reads from its call: the transport, the display width, the concurrency cap and the logging context.
pub struct DispatchSettings<'settings> {
    pub provider: &'settings dyn ProviderHealth,
    pub details: bool,
    pub max_concurrency: Option<i64>,
    pub instrumentation: &'settings InstrumentationContext,
}

// _perform_health_check: dispatch every prepared probe, instrument the dispatch, then split the answers by outcome
#[must_use]
pub async fn perform_health_checks(
    checked: &[DeploymentSpec],
    prepared: &[PreparedProbe],
    settings: &DispatchSettings<'_>,
) -> HealthCheckResult {
    let (outcomes, mode, peak_in_flight) =
        run_probes(prepared, settings.provider, settings.max_concurrency).await;
    dispatch_summary(
        settings.instrumentation,
        mode,
        checked.len(),
        settings.max_concurrency,
        peak_in_flight,
    );
    separate_results(checked, &outcomes, settings.details)
}
