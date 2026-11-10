use super::contracts::SupportsRawQueries;
pub const SPEND_LOGS_ESTIMATE_SQL: &str = r#"SELECT reltuples::BIGINT
FROM pg_class
WHERE oid = '"LiteLLM_SpendLogs"'::regclass;"#;
pub async fn should_create_missing_views<Database>(
    database: &Database,
) -> Result<bool, Database::Error>
where
    Database: SupportsRawQueries + Sync,
{
    let rows = database.query_raw(SPEND_LOGS_ESTIMATE_SQL).await?;
    let estimate = rows
        .first()
        .and_then(|row| row.columns.get("reltuples").copied().flatten());
    tracing::debug!("Estimated Row count of LiteLLM_SpendLogs = {estimate:?}");
    let enabled = matches!(estimate, Some(0 | -1));
    if enabled {
        tracing::debug!("Should create views");
    }
    Ok(enabled)
}
