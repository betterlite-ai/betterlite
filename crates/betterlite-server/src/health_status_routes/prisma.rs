use betterlite_db::health_check_rows::HealthCheckRows;

use crate::health_status_routes::error::HealthRouteError;

// ast-grep-ignore: no-one-line-functions - _check_prisma_client, which the task requires and both routes call
pub fn check_prisma_client(
    rows: Option<&HealthCheckRows>,
) -> Result<&HealthCheckRows, HealthRouteError> {
    rows.ok_or(HealthRouteError::Database)
}
