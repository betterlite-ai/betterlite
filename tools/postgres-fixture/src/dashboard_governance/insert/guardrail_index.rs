use tokio_postgres::Client;
use tokio_postgres::types::ToSql;

use super::execute;
use crate::dashboard_governance::DashboardGovernanceError;
use crate::dashboard_governance::model::GuardrailIndexRow;

const GUARDRAIL_INDEX_INSERT: &str = "INSERT INTO \"LiteLLM_SpendLogGuardrailIndex\" (\"request_id\",\"guardrail_id\",\"policy_id\",\"start_time\") VALUES ($1,$2,$3,$4::text::timestamp(3))";

pub(super) async fn insert(
    client: &Client,
    guardrail_index: &[GuardrailIndexRow],
) -> Result<(), DashboardGovernanceError> {
    for (row, index) in guardrail_index.iter().enumerate() {
        let values: &[&(dyn ToSql + Sync)] = &[
            &index.request_id,
            &index.guardrail_id,
            &index.policy_id,
            &index.start_time,
        ];
        execute(
            client,
            "guardrail_index",
            row,
            GUARDRAIL_INDEX_INSERT,
            values,
        )
        .await?;
    }
    Ok(())
}
