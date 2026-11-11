use betterlite_config::sections::io_token_reservations as config;
use betterlite_router::io_token_reservations::limits::IoLimits;

mod tests;

// LiteLLM keeps one deployment dict per model_list entry, so the resolved limits stay index-aligned
#[must_use]
pub fn deployment_limits(section: &config::Settings) -> Vec<IoLimits> {
    let mut limits = Vec::with_capacity(section.deployments.len());
    for deployment in &section.deployments {
        limits.push(IoLimits {
            itpm: deployment.itpm,
            otpm: deployment.otpm,
        });
    }
    limits
}
