use super::{DailyRowFilter, bind_params, render_filters};
use crate::daily_activity_contracts::identifiers::DailyActivityTable;
use crate::daily_activity_contracts::{DailyActivityRow, SqlParameter};
use crate::daily_activity_repository::rows::RowError;
use crate::daily_activity_repository::rows::records::daily_activity_row;
use crate::postgres_store::errors::{DbError, backend};
use deadpool_postgres::Client;

pub async fn find_daily_query(
    client: &Client,
    table: DailyActivityTable,
    filters: &[DailyRowFilter],
    skip: i64,
    take: i64,
) -> Result<Vec<DailyActivityRow>, DbError<RowError>> {
    let (where_sql, mut parameters) = render_filters(table, filters);
    let limit_index = parameters.len().saturating_add(1);
    let offset_index = parameters.len().saturating_add(2);
    parameters.push(SqlParameter::Integer(take));
    parameters.push(SqlParameter::Integer(skip));
    let relation = table.metadata().relation;
    let sql = format!(
        "SELECT * FROM \"{relation}\" WHERE {where_sql} ORDER BY date DESC, id ASC LIMIT ${limit_index} OFFSET ${offset_index}"
    );
    let params = bind_params(&parameters);
    let rows = client.query(sql.as_str(), &params).await.map_err(backend)?;
    let mut decoded = Vec::with_capacity(rows.len());
    for row in &rows {
        decoded.push(daily_activity_row(row).map_err(|source| DbError::InvalidRecord { source })?);
    }
    Ok(decoded)
}
