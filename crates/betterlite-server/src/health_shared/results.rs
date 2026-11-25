use std::sync::{Mutex, PoisonError};

use crate::health_cycle::endpoints::HealthCheckResult;
use crate::health_status_routes::state::{HealthCheckResults, HealthResultsSource};

// health_check_results as the background loop leaves it: the four keys GET /health answers from the cache.
#[derive(Debug, Default)]
pub struct HealthResultsCache {
    results: Mutex<HealthCheckResults>,
}

impl HealthResultsCache {
    // The source fills the global right after a cycle's check, before the database save.
    pub fn publish(&self, result: &HealthCheckResult) {
        let mut results = self.results.lock().unwrap_or_else(PoisonError::into_inner);
        *results = HealthCheckResults {
            healthy_endpoints: result.healthy_endpoints.clone(),
            unhealthy_endpoints: result.unhealthy_endpoints.clone(),
            healthy_count: result.healthy_endpoints.len(),
            unhealthy_count: result.unhealthy_endpoints.len(),
        };
    }
}

impl HealthResultsSource for HealthResultsCache {
    fn snapshot(&self) -> HealthCheckResults {
        let results = self.results.lock().unwrap_or_else(PoisonError::into_inner);
        results.clone()
    }
}
