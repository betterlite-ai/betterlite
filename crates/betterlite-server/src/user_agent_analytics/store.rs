use async_trait::async_trait;
use betterlite_db::postgres_store::errors::backend;
use betterlite_db::postgres_store::session::PostgresStore;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

use super::error::AnalyticsError;
use super::query::TagFilter;
use super::rows::{
    ActiveUsersReport, ActiveUsersRow, DistinctTagRow, TagSpendRecord, TagSummaryRow,
    TokenOwnerRow, UserEmailRow,
};

pub const MAX_TAGS: i64 = 250;

const FIRST_TAG_PARAMETER: usize = 3;
const CONDITION_MARKER: &str = "/*conditions*/";

#[derive(Clone, Copy)]
pub struct RangeQuery<'query> {
    pub start_date: &'query str,
    pub end_date: &'query str,
    pub filter: Option<&'query TagFilter>,
}

#[derive(Clone, Copy)]
enum TagMatching {
    Sensitive,
    Insensitive,
}

const DISTINCT_TAGS_SQL: &str = "SELECT dts.tag, COUNT(*) AS usage_count FROM \"LiteLLM_DailyTagSpend\" dts WHERE dts.tag LIKE 'User-Agent:%' OR dts.tag NOT LIKE '%:%' GROUP BY dts.tag ORDER BY usage_count DESC LIMIT $1";

const DAILY_ACTIVE_USERS_SQL: &str = "SELECT dts.tag, dts.date, COUNT(DISTINCT vt.user_id) AS active_users FROM \"LiteLLM_DailyTagSpend\" dts INNER JOIN \"LiteLLM_VerificationToken\" vt ON dts.api_key = vt.token WHERE dts.date >= $1 AND dts.date <= $2 AND vt.user_id IS NOT NULL/*conditions*/ GROUP BY dts.tag, dts.date ORDER BY dts.date DESC, active_users DESC";

// the outer filter keeps the seven MAX_WEEKS buckets the WAU labels number, as LiteLLM's inline constant does
const WEEKLY_ACTIVE_USERS_SQL: &str = "WITH weekly_data AS (SELECT dts.tag, vt.user_id, ($2::text::date - dts.date::date) / 7 AS week_offset FROM \"LiteLLM_DailyTagSpend\" dts INNER JOIN \"LiteLLM_VerificationToken\" vt ON dts.api_key = vt.token WHERE dts.date >= $1 AND dts.date <= $2 AND vt.user_id IS NOT NULL/*conditions*/) SELECT tag, COUNT(DISTINCT user_id) AS active_users, week_offset::text AS date FROM weekly_data WHERE week_offset < 7 GROUP BY tag, week_offset ORDER BY week_offset DESC, active_users DESC";

// integer division of a non-negative day difference truncates, which is FLOOR for the 30-day month buckets
const MONTHLY_ACTIVE_USERS_SQL: &str = "WITH monthly_data AS (SELECT dts.tag, vt.user_id, ($2::text::date - dts.date::date) / 30 AS month_offset FROM \"LiteLLM_DailyTagSpend\" dts INNER JOIN \"LiteLLM_VerificationToken\" vt ON dts.api_key = vt.token WHERE dts.date >= $1 AND dts.date <= $2 AND vt.user_id IS NOT NULL/*conditions*/) SELECT tag, COUNT(DISTINCT user_id) AS active_users, month_offset::text AS date FROM monthly_data WHERE month_offset < 7 GROUP BY tag, month_offset ORDER BY month_offset DESC, active_users DESC";

const TAG_SUMMARY_SQL: &str = "SELECT dts.tag, COUNT(DISTINCT vt.user_id) AS unique_users, SUM(dts.api_requests)::bigint AS total_requests, SUM(dts.successful_requests)::bigint AS successful_requests, SUM(dts.failed_requests)::bigint AS failed_requests, SUM(dts.prompt_tokens + dts.completion_tokens)::bigint AS total_tokens, SUM(dts.spend)::numeric AS total_spend FROM \"LiteLLM_DailyTagSpend\" dts LEFT JOIN \"LiteLLM_VerificationToken\" vt ON dts.api_key = vt.token WHERE dts.date >= $1 AND dts.date <= $2/*conditions*/ GROUP BY dts.tag ORDER BY total_requests DESC";

const TAG_RECORDS_SQL: &str = "SELECT dts.tag, dts.api_key, dts.api_requests, dts.successful_requests, dts.failed_requests, dts.prompt_tokens, dts.completion_tokens, dts.spend::numeric AS spend FROM \"LiteLLM_DailyTagSpend\" dts WHERE dts.date >= $1 AND dts.date <= $2 AND dts.api_key <> ''/*conditions*/ ORDER BY dts.date ASC, dts.id ASC";

const TOKEN_OWNERS_SQL: &str = "SELECT vt.token, vt.user_id FROM \"LiteLLM_VerificationToken\" vt WHERE vt.token = ANY($1::text[]) AND vt.user_id IS NOT NULL AND vt.user_id <> ''";

const USER_EMAILS_SQL: &str =
    "SELECT u.user_id, u.user_email FROM \"LiteLLM_UserTable\" u WHERE u.user_id = ANY($1::text[])";

#[async_trait]
pub trait UserAgentAnalyticsStore: Send + Sync {
    async fn distinct_tags(&self) -> Result<Vec<DistinctTagRow>, AnalyticsError>;

    async fn active_users(
        &self,
        report: ActiveUsersReport,
        query: RangeQuery<'_>,
    ) -> Result<Vec<ActiveUsersRow>, AnalyticsError>;

    async fn tag_summary(
        &self,
        query: RangeQuery<'_>,
    ) -> Result<Vec<TagSummaryRow>, AnalyticsError>;

    async fn tag_records(
        &self,
        query: RangeQuery<'_>,
    ) -> Result<Vec<TagSpendRecord>, AnalyticsError>;

    async fn token_owners(&self, tokens: &[String]) -> Result<Vec<TokenOwnerRow>, AnalyticsError>;

    async fn user_emails(&self, user_ids: &[String]) -> Result<Vec<UserEmailRow>, AnalyticsError>;
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

    async fn decoded_rows<Decoded>(
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

    async fn filtered_rows<Decoded>(
        &self,
        template: &str,
        query: RangeQuery<'_>,
        matching: TagMatching,
    ) -> Result<Vec<Decoded>, AnalyticsError>
    where
        Decoded: for<'row> TryFrom<&'row Row, Error = AnalyticsError>,
    {
        let (sql, values) = filtered_sql(template, query, matching);
        self.decoded_rows(&sql, &values).await
    }
}

const fn active_users_sql(report: ActiveUsersReport) -> &'static str {
    match report {
        ActiveUsersReport::Daily => DAILY_ACTIVE_USERS_SQL,
        ActiveUsersReport::Weekly => WEEKLY_ACTIVE_USERS_SQL,
        ActiveUsersReport::Monthly => MONTHLY_ACTIVE_USERS_SQL,
    }
}

fn filtered_sql(
    template: &str,
    query: RangeQuery<'_>,
    matching: TagMatching,
) -> (String, Vec<String>) {
    let (conditions, values) = tag_conditions(query.filter, matching);
    let mut params = vec![query.start_date.to_owned(), query.end_date.to_owned()];
    params.extend(values);
    (template.replace(CONDITION_MARKER, &conditions), params)
}

fn tag_conditions(filter: Option<&TagFilter>, matching: TagMatching) -> (String, Vec<String>) {
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
        Some(TagFilter::Substring(text)) => {
            let operator = match matching {
                TagMatching::Sensitive => "LIKE",
                TagMatching::Insensitive => "ILIKE",
            };
            (
                format!(" AND dts.tag {operator} ${FIRST_TAG_PARAMETER}"),
                vec![format!("%{text}%")],
            )
        }
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
        query: RangeQuery<'_>,
    ) -> Result<Vec<ActiveUsersRow>, AnalyticsError> {
        self.filtered_rows(active_users_sql(report), query, TagMatching::Insensitive)
            .await
    }

    async fn tag_summary(
        &self,
        query: RangeQuery<'_>,
    ) -> Result<Vec<TagSummaryRow>, AnalyticsError> {
        self.filtered_rows(TAG_SUMMARY_SQL, query, TagMatching::Insensitive)
            .await
    }

    async fn tag_records(
        &self,
        query: RangeQuery<'_>,
    ) -> Result<Vec<TagSpendRecord>, AnalyticsError> {
        self.filtered_rows(TAG_RECORDS_SQL, query, TagMatching::Sensitive)
            .await
    }

    async fn token_owners(&self, tokens: &[String]) -> Result<Vec<TokenOwnerRow>, AnalyticsError> {
        let owned = tokens.to_vec();
        let rows = self.fetch(TOKEN_OWNERS_SQL, &[&owned]).await?;
        rows.iter().map(TokenOwnerRow::try_from).collect()
    }

    async fn user_emails(&self, user_ids: &[String]) -> Result<Vec<UserEmailRow>, AnalyticsError> {
        let owned = user_ids.to_vec();
        let rows = self.fetch(USER_EMAILS_SQL, &[&owned]).await?;
        rows.iter().map(UserEmailRow::try_from).collect()
    }
}
