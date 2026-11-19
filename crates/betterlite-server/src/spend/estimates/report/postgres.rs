use std::str::FromStr as _;

use async_trait::async_trait;
use betterlite_db::postgres_store::errors::backend;
use betterlite_db::postgres_store::session::PostgresStore;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

use super::{ReportError, SpendReportSource, TagSpendRow, TeamSpendRow};

// _get_daily_spend_reports' team query: a tz-aware parameter wrapped back to UTC, then the inclusive end day.
const TEAM_SQL: &str = r#"SELECT t.team_alias, SUM(s.spend)::text AS spend_text
FROM "LiteLLM_SpendLogs" s
LEFT JOIN "LiteLLM_TeamTable" t ON s.team_id = t.team_id
WHERE s."startTime" >= ($1::timestamptz AT TIME ZONE 'UTC')
  AND s."startTime" < (($2::timestamptz + INTERVAL '1 day') AT TIME ZONE 'UTC')
GROUP BY t.team_alias
ORDER BY SUM(s.spend) DESC"#;

// The same window with one row per request tag, as jsonb_array_elements_text expands them.
const TAG_SQL: &str = r#"SELECT jsonb_array_elements_text(request_tags) AS individual_request_tag, SUM(spend)::text AS spend_text
FROM "LiteLLM_SpendLogs"
WHERE "startTime" >= ($1::timestamptz AT TIME ZONE 'UTC')
  AND "startTime" < (($2::timestamptz + INTERVAL '1 day') AT TIME ZONE 'UTC')
GROUP BY individual_request_tag
ORDER BY SUM(spend) DESC"#;

pub struct PostgresSpendReportStore {
    pub store: PostgresStore,
}

impl PostgresSpendReportStore {
    async fn read<Record>(
        &self,
        sql: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<Record>, ReportError>
    where
        Record: for<'row> TryFrom<&'row Row, Error = ReportError>,
    {
        let client = self.store.session().await?;
        let params: [&(dyn ToSql + Sync); 2] = [&start, &end];
        let rows = client
            .query(sql, &params)
            .await
            .map_err(|source| ReportError::Database(backend(source)))?;
        let mut records = Vec::with_capacity(rows.len());
        for row in &rows {
            records.push(Record::try_from(row)?);
        }
        Ok(records)
    }
}

#[async_trait]
impl SpendReportSource for PostgresSpendReportStore {
    async fn team_totals(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<TeamSpendRow>, ReportError> {
        self.read(TEAM_SQL, start, end).await
    }

    async fn tag_totals(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<TagSpendRow>, ReportError> {
        self.read(TAG_SQL, start, end).await
    }
}

impl TryFrom<&Row> for TeamSpendRow {
    type Error = ReportError;

    fn try_from(row: &Row) -> Result<Self, Self::Error> {
        Ok(Self {
            team_alias: text(row, "team_alias")?,
            total_spend: spend(row)?,
        })
    }
}

impl TryFrom<&Row> for TagSpendRow {
    type Error = ReportError;

    fn try_from(row: &Row) -> Result<Self, Self::Error> {
        Ok(Self {
            request_tag: text(row, "individual_request_tag")?,
            total_spend: spend(row)?,
        })
    }
}

fn text(row: &Row, column: &'static str) -> Result<Option<String>, ReportError> {
    row.try_get(column)
        .map_err(|source| ReportError::Database(backend(source)))
}

// SUM is double precision, so the query casts it to text and the row parses it into a Decimal.
fn spend(row: &Row) -> Result<Decimal, ReportError> {
    let text: String = row
        .try_get("spend_text")
        .map_err(|source| ReportError::Database(backend(source)))?;
    Decimal::from_str(&text).map_err(|source| ReportError::Spend {
        column: "spend_text",
        source,
    })
}
