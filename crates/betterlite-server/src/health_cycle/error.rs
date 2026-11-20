use crate::health::router::error::RouterHealthError;

// perform_health_check raises the catalog lookup a dependency resolves through, and nothing else escapes the cycle
#[derive(Debug, thiserror::Error)]
pub enum HealthCycleError {
    #[error(transparent)]
    Router(#[from] RouterHealthError),
}
