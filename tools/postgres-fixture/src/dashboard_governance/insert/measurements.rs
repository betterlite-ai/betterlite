use tokio_postgres::Client;
use tokio_postgres::types::ToSql;

use super::{CREATED_AT, execute};
use crate::dashboard_governance::DashboardGovernanceError;
use crate::dashboard_governance::model::GuardrailMetricRow;

const METRIC_INSERT: &str = "INSERT INTO \"LiteLLM_DailyGuardrailMetrics\" (\"guardrail_id\",\"date\",\"requests_evaluated\",\"passed_count\",\"blocked_count\",\"flagged_count\",\"avg_score\",\"avg_latency_ms\",\"created_at\",\"updated_at\") VALUES ($1,$2,$3,$4,$5,$6,$7::text::numeric::double precision,$8::text::numeric::double precision,$9::text::timestamp(3),$10::text::timestamp(3))";

pub(super) async fn insert(
    client: &Client,
    metrics: &[GuardrailMetricRow],
) -> Result<(), DashboardGovernanceError> {
    for (row, metric) in metrics.iter().enumerate() {
        let avg_score = metric.avg_score.as_ref().map(ToString::to_string);
        let avg_latency = metric.avg_latency_ms.as_ref().map(ToString::to_string);
        let values: &[&(dyn ToSql + Sync)] = &[
            &metric.guardrail_id,
            &metric.date,
            &metric.requests_evaluated,
            &metric.passed_count,
            &metric.blocked_count,
            &metric.flagged_count,
            &avg_score,
            &avg_latency,
            &CREATED_AT,
            &CREATED_AT,
        ];
        execute(client, "metrics", row, METRIC_INSERT, values).await?;
    }
    Ok(())
}
