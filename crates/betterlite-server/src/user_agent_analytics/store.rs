use async_trait::async_trait;
use betterlite_db::postgres_store::errors::backend;
use betterlite_db::postgres_store::session::PostgresStore;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

use super::error::AnalyticsError;
use super::query::TagFilter;
use super::rows::{ActiveUsersReport, ActiveUsersRow, DistinctTagRow};

pub const MAX_TAGS: i64 = 250;

const FIRST_TAG_PARAMETER: usize = 3;
const CONDITION_MARKER: &str = "/*conditions*/";

const DISTINCT_TAGS_SQL: &str = "SELECT dts.tag, COUNT(*) AS usage_count FROM \"LiteLLM_DailyTagSpend\" dts WHERE dts.tag LIKE 'User-Agent:%' OR dts.tag NOT LIKE '%:%' GROUP BY dts.tag ORDER BY usage_count DESC LIMIT $1";

const DAILY_ACTIVE_USERS_SQL: &str = "SELECT dts.tag, dts.date, COUNT(DISTINCT vt.user_id) AS active_users FROM \"LiteLLM_DailyTagSpend\" dts INNER JOIN \"LiteLLM_VerificationToken\" vt ON dts.api_key = vt.token WHERE dts.date >= $1 AND dts.date <= $2 AND vt.user_id IS NOT NULL/*conditions*/ GROUP BY dts.tag, dts.date ORDER BY dts.date DESC, active_users DESC";

// the outer filter keeps the seven MAX_WEEKS buckets the WAU labels number, as LiteLLM's inline constant does
const WEEKLY_ACTIVE_USERS_SQL: &str = "WITH weekly_data AS (SELECT dts.tag, vt.user_id, ($2::text::date - dts.date::date) / 7 AS week_offset FROM \"LiteLLM_DailyTagSpend\" dts INNER JOIN \"LiteLLM_VerificationToken\" vt ON dts.api_key = vt.token WHERE dts.date >= $1 AND dts.date <= $2 AND vt.user_id IS NOT NULL/*conditions*/) SELECT tag, COUNT(DISTINCT user_id) AS active_users, week_offset::text AS date FROM weekly_data WHERE week_offset < 7 GROUP BY tag, week_offset ORDER BY week_offset DESC, active_users DESC";

#[async_trait]
pub trait UserAgentAnalyticsStore: Send + Sync {
    async fn distinct_tags(&self) -> Result<Vec<DistinctTagRow>, AnalyticsError>;

    async fn active_users(
        &self,
        report: ActiveUsersReport,
        start_date: &str,
        end_date: &str,
        filter: Option<&TagFilter>,
    ) -> Result<Vec<ActiveUsersRow>, AnalyticsError>;
}

pub struct PostgresAnalyticsStore {
    pub store: PostgresStore,
}

impl PostgresAnalyticsStore {
    async fn fetch(
        &self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Vec<Row>, AnalyticsError> {
        let client = self.store.session().await?;
        client
            .query(sql, params)
            .await
            .map_err(|source| AnalyticsError::Database(backend(source)))
    }

    async fn active_rows<Decoded>(
        &self,
        sql: &str,
        values: &[String],
    ) -> Result<Vec<Decoded>, AnalyticsError>
    where
        Decoded: for<'row> TryFrom<&'row Row, Error = AnalyticsError>,
    {
        let mut params: Vec<&(dyn ToSql + Sync)> = Vec::new();
        for value in values {
            params.push(value);
        }
        let rows = self.fetch(sql, &params).await?;
        rows.iter().map(Decoded::try_from).collect()
    }
}

fn active_users_sql(
    template: &str,
    start_date: &str,
    end_date: &str,
    filter: Option<&TagFilter>,
) -> (String, Vec<String>) {
    let (conditions, values) = tag_conditions(filter);
    let mut params = vec![start_date.to_owned(), end_date.to_owned()];
    params.extend(values);
    (template.replace(CONDITION_MARKER, &conditions), params)
}

fn tag_conditions(filter: Option<&TagFilter>) -> (String, Vec<String>) {
    match filter {
        Some(TagFilter::Exact(tags)) => {
            let mut conditions = Vec::new();
            let mut values = Vec::new();
            for (tag, position) in tags.iter().zip(FIRST_TAG_PARAMETER..) {
                conditions.push(format!("dts.tag = ${position}"));
                values.push(tag.clone());
            }
            (format!(" AND ({})", conditions.join(" OR ")), values)
        }
        Some(TagFilter::Substring(text)) => (
            format!(" AND dts.tag ILIKE ${FIRST_TAG_PARAMETER}"),
            vec![format!("%{text}%")],
        ),
        None => (String::new(), Vec::new()),
    }
}

#[async_trait]
impl UserAgentAnalyticsStore for PostgresAnalyticsStore {
    async fn distinct_tags(&self) -> Result<Vec<DistinctTagRow>, AnalyticsError> {
        let rows = self.fetch(DISTINCT_TAGS_SQL, &[&MAX_TAGS]).await?;
        rows.iter().map(DistinctTagRow::try_from).collect()
    }

    async fn active_users(
        &self,
        report: ActiveUsersReport,
        start_date: &str,
        end_date: &str,
        filter: Option<&TagFilter>,
    ) -> Result<Vec<ActiveUsersRow>, AnalyticsError> {
        let template = match report {
            ActiveUsersReport::Daily => DAILY_ACTIVE_USERS_SQL,
            ActiveUsersReport::Weekly => WEEKLY_ACTIVE_USERS_SQL,
        };
        let (sql, values) = active_users_sql(template, start_date, end_date, filter);
        self.active_rows(&sql, &values).await
    }
}
