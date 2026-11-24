use betterlite_db::health_check_rows::HealthCheckRows;

use crate::health_status_routes::error::HealthRouteError;

// _check_prisma_client: without the store the endpoint answers 500 before it reads anything
pub fn check_prisma_client(
    rows: Option<&HealthCheckRows>,
) -> Result<&HealthCheckRows, HealthRouteError> {
    rows.ok_or(HealthRouteError::Database)
}
