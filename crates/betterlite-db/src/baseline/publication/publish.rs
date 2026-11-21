use deadpool_postgres::GenericClient;
use rust_decimal::Decimal;
use uuid::Uuid;

use super::change::Change;
use super::sql::{UPDATE_LOGS, UPDATE_PUBLICATIONS, session_correction_sql};
use crate::daily_spend_sql::bind::{SqlBind, execute};
use crate::daily_spend_sql::entities::{DailySpendEntity, SpendRow};
use crate::daily_spend_sql::write::{DailyWriteError, write_bulk_upsert};

// DAILY_SPEND_TABLES' order: the statements run entity by entity, tag last.
const ENTITIES: [DailySpendEntity; 6] = [
    DailySpendEntity::User,
    DailySpendEntity::Team,
    DailySpendEntity::Org,
    DailySpendEntity::EndUser,
    DailySpendEntity::Agent,
    DailySpendEntity::Tag,
];

#[derive(Debug, thiserror::Error)]
pub enum PublishError {
    #[error("baseline change payload cannot be represented as JSON")]
    Json(#[source] serde_json::Error),
    #[error("baseline correction statement failed")]
    Statement(#[source] tokio_postgres::Error),
    #[error("daily savings adjustment failed")]
    Daily(#[from] DailyWriteError),
}

// _publish: logs, key sessions, user sessions, daily savings, then the stored publications, on one transaction.
pub async fn publish<C, G>(
    client: &C,
    changes: &[Change],
    mut generate_id: G,
) -> Result<(), PublishError>
where
    C: GenericClient + Sync,
    G: FnMut() -> Uuid,
{
    if changes.is_empty() {
        return Ok(());
    }
    let payload = serde_json::to_string(changes).map_err(PublishError::Json)?;
    let params = [SqlBind::Text(Some(payload))];
    execute(client, UPDATE_LOGS, &params)
        .await
        .map_err(PublishError::Statement)?;
    execute(client, &session_correction_sql(false), &params)
        .await
        .map_err(PublishError::Statement)?;
    if changes.iter().any(|change| !change.user_id.is_empty()) {
        execute(client, &session_correction_sql(true), &params)
            .await
            .map_err(PublishError::Statement)?;
    }
    for entity in ENTITIES {
        let rows = daily_rows(changes, entity);
        write_bulk_upsert(client, entity, &rows, &mut generate_id).await?;
    }
    execute(client, UPDATE_PUBLICATIONS, &params)
        .await
        .map_err(PublishError::Statement)?;
    Ok(())
}

// Only a nonzero savings delta adjusts a daily row; duplicate targets stay duplicate rows until the merge folds them.
fn daily_rows(changes: &[Change], entity: DailySpendEntity) -> Vec<SpendRow> {
    let mut rows = Vec::new();
    for change in changes {
        if change.savings_delta == Decimal::ZERO {
            continue;
        }
        let Some(daily) = &change.daily else {
            continue;
        };
        for target in &daily.targets {
            if DailySpendEntity::from(target.entity) == entity {
                rows.push(daily.adjustment(target, change.savings_delta, &change.request_id));
            }
        }
    }
    rows
}
