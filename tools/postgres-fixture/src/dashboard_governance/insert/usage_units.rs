use tokio_postgres::Client;
use tokio_postgres::types::ToSql;

use super::{CREATED_AT, execute, measured_key_hash};
use crate::dashboard_governance::DashboardGovernanceError;
use crate::dashboard_governance::model::UsageUnitRow;
use crate::seed::FixtureSeed;

const USAGE_UNIT_INSERT: &str = "INSERT INTO \"LiteLLM_DailyGuardrailUsageUnits\" (\"guardrail_id\",\"date\",\"team_id\",\"api_key\",\"usage_unit\",\"units\",\"cost\",\"untracked_units\",\"created_at\",\"updated_at\") VALUES ($1,$2,$3,$4,$5,$6,$7::text::numeric::double precision,$8,$9::text::timestamp(3),$10::text::timestamp(3))";

pub(super) async fn insert(
    client: &Client,
    usage_units: &[UsageUnitRow],
    fixture: &FixtureSeed,
) -> Result<(), DashboardGovernanceError> {
    for (row, unit) in usage_units.iter().enumerate() {
        let api_key = match unit.key_index {
            Some(index) => measured_key_hash("usage_units", row, index, fixture)?,
            None => String::new(),
        };
        let cost = unit.cost.as_ref().map(ToString::to_string);
        let values: &[&(dyn ToSql + Sync)] = &[
            &unit.guardrail_id,
            &unit.date,
            &unit.team_id,
            &api_key,
            &unit.usage_unit,
            &unit.units,
            &cost,
            &unit.untracked_units,
            &CREATED_AT,
            &CREATED_AT,
        ];
        execute(client, "usage_units", row, USAGE_UNIT_INSERT, values).await?;
    }
    Ok(())
}
