use betterlite_db::health_check_rows::HealthCheckRow;
use betterlite_models::model_registration::JsonObject;
use chrono::{DateTime, Utc};

use super::LatestChecks;

// the map _save_background_health_checks_to_db builds: the id when it has text, else the name, and the first row for a key wins
#[must_use]
pub fn latest_by_key(rows: Vec<HealthCheckRow>) -> LatestChecks {
    let mut latest = LatestChecks::new();
    for row in rows {
        let key = match &row.model_id {
            Some(id) if !id.is_empty() => id.clone(),
            Some(_) | None => row.model_name.clone(),
        };
        latest.entry(key).or_insert(row);
    }
    latest
}

// (time.time() - start_time) * 1000, measured on the injected clock
pub(super) fn elapsed_ms(now: DateTime<Utc>, start: DateTime<Utc>) -> f64 {
    now.signed_duration_since(start)
        .as_seconds_f64()
        .mul_add(1000.0_f64, 0.0_f64)
}

// _save_health_check_to_db's status: any healthy endpoint makes the model healthy
pub(super) fn endpoint_status(healthy: &[JsonObject]) -> &'static str {
    if healthy.is_empty() {
        return "unhealthy";
    }
    "healthy"
}
