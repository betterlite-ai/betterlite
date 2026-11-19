use crate::daily_activity_contracts::SqlParameter;
use crate::gateway_requests::GatewayRequestSnapshot;

mod tests;

pub const GATEWAY_REQUESTS_TABLE: &str = "\"LiteLLM_DailyGatewayRequests\"";
const COLUMNS_PER_ROW: usize = 5;
const UTC_NOW: &str = "(NOW() AT TIME ZONE 'UTC')";

#[must_use]
pub fn build_gateway_requests_upsert(
    snapshot: &GatewayRequestSnapshot,
) -> (String, Vec<SqlParameter>) {
    let mut rows = Vec::with_capacity(snapshot.len());
    let mut parameters = Vec::with_capacity(snapshot.len().saturating_mul(COLUMNS_PER_ROW));
    for (index, (key, counts)) in snapshot.iter().enumerate() {
        let base = index.saturating_mul(COLUMNS_PER_ROW);
        rows.push(format!(
            "(${}::text, ${}::text, ${}::text, ${}::bigint, ${}::bigint, {UTC_NOW})",
            base.saturating_add(1),
            base.saturating_add(2),
            base.saturating_add(3),
            base.saturating_add(4),
            base.saturating_add(5)
        ));
        parameters.extend([
            SqlParameter::Text(key.date.clone()),
            SqlParameter::Text(key.category.clone()),
            SqlParameter::Text(key.route.clone()),
            SqlParameter::Integer(counts.successful_requests),
            SqlParameter::Integer(counts.failed_requests),
        ]);
    }
    let sql = format!(
        "INSERT INTO {GATEWAY_REQUESTS_TABLE} (\"date\", \"category\", \"route\", \"successful_requests\", \"failed_requests\", \"updated_at\")\nVALUES {}\nON CONFLICT (\"date\", \"category\", \"route\") DO UPDATE SET\n  \"successful_requests\" = {GATEWAY_REQUESTS_TABLE}.\"successful_requests\" + EXCLUDED.\"successful_requests\",\n  \"failed_requests\" = {GATEWAY_REQUESTS_TABLE}.\"failed_requests\" + EXCLUDED.\"failed_requests\",\n  \"updated_at\" = {UTC_NOW}",
        rows.join(", ")
    );
    (sql, parameters)
}
