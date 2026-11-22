use std::sync::Arc;

use async_trait::async_trait;
use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::postgres_store::errors::{DbError, backend};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::table_ops::rows::{RowDecoder, RowError, UserWithAliases};
use rust_decimal::Decimal;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

use super::rows::{DailyTagSpend, KeyAliasedUser, RawTagSpend};

// B722's find_all projection, ordered by spend as get_data(table_name="key", query_type="find_all")
const KEYS_SQL: &str = "SELECT row_to_json(x)::text FROM (SELECT v.*, to_jsonb(b.*) AS litellm_budget_table FROM \"LiteLLM_VerificationToken\" v LEFT JOIN \"LiteLLM_BudgetTable\" b ON v.\"budget_id\" = b.\"budget_id\" ORDER BY v.\"spend\" DESC) x";
// B722's find_many(where={"user_id": user_id}) projection, which carries no ORDER BY
const USER_KEYS_SQL: &str = "SELECT row_to_json(x)::text FROM (SELECT v.*, to_jsonb(b.*) AS litellm_budget_table FROM \"LiteLLM_VerificationToken\" v LEFT JOIN \"LiteLLM_BudgetTable\" b ON v.\"budget_id\" = b.\"budget_id\" WHERE v.\"user_id\" = $1) x";
// B723's users_all projection: u.* with the key aliases, ordered by stored spend
const USERS_SQL: &str = "SELECT row_to_json(x)::text FROM (SELECT u.*, json_agg(v.key_alias) AS key_aliases FROM \"LiteLLM_UserTable\" u LEFT JOIN \"LiteLLM_VerificationToken\" v ON u.user_id = v.user_id GROUP BY u.user_id ORDER BY u.spend DESC LIMIT $1 OFFSET $2) x";
// B723's user_unique projection, with membership rows included the way get_data names them
const USER_SQL: &str = "SELECT row_to_json(x)::text FROM (SELECT u.*, COALESCE((SELECT json_agg(m.*) FROM \"LiteLLM_OrganizationMembership\" m WHERE m.\"user_id\" = u.\"user_id\"), '[]'::json) AS organization_memberships FROM \"LiteLLM_UserTable\" u WHERE u.\"user_id\" = $1) x";
const RAW_TAGS_SQL: &str = "SELECT jsonb_array_elements_text(request_tags) AS individual_request_tag, COUNT(*) AS log_count, SUM(spend)::text AS spend_text FROM \"LiteLLM_SpendLogs\" GROUP BY individual_request_tag";
const TAG_NAMES_SQL: &str = "SELECT DISTINCT jsonb_array_elements_text(request_tags) AS individual_request_tag FROM \"LiteLLM_SpendLogs\"";
const DAILY_ALL_TAGS_SQL: &str = "SELECT individual_request_tag, spend_date, log_count::bigint AS log_count, total_spend::text AS spend_text FROM \"DailyTagSpend\" WHERE spend_date >= $1::date AND spend_date <= $2::date ORDER BY total_spend DESC";
const DAILY_TAGS_SQL: &str = "SELECT individual_request_tag, SUM(log_count)::bigint AS log_count, SUM(total_spend)::text AS spend_text FROM \"DailyTagSpend\" WHERE spend_date >= $1::date AND spend_date <= $2::date AND individual_request_tag = ANY($3::text[]) GROUP BY individual_request_tag ORDER BY SUM(total_spend) DESC";
const SPEND_TEXT_COLUMN: &str = "spend_text";
// get_data passes limit=None and offset=None, which PostgreSQL reads as LIMIT ALL OFFSET 0
const NO_LIMIT: Option<i64> = None;

pub type TagReportHandle = Option<Arc<dyn TagReportStore>>;

#[derive(Debug, thiserror::Error)]
pub enum TagReportError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error(transparent)]
    Row(#[from] RowError),
    #[error("the database returned a null value for the {column} column")]
    NullAmount { column: &'static str },
    #[error("the database returned an unreadable value for the {column} column")]
    UnreadableAmount {
        column: &'static str,
        #[source]
        source: rust_decimal::Error,
    },
}

#[async_trait]
pub trait TagReportStore: Send + Sync {
    async fn keys(&self) -> Result<Vec<LiteLLMVerificationToken>, TagReportError>;
    async fn keys_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<LiteLLMVerificationToken>, TagReportError>;
    async fn users(&self) -> Result<Vec<KeyAliasedUser>, TagReportError>;
    async fn user(&self, user_id: &str) -> Result<Option<LiteLLMUserTable>, TagReportError>;
    async fn raw_tag_spend(&self) -> Result<Vec<RawTagSpend>, TagReportError>;
    // _query_raw_or_none: None stands for a driver that answered no result at all
    async fn tag_names(&self) -> Result<Option<Vec<Option<String>>>, TagReportError>;
    async fn daily_tag_spend(
        &self,
        start_date: &str,
        end_date: &str,
        tags: Option<&[String]>,
    ) -> Result<Vec<DailyTagSpend>, TagReportError>;
}

pub struct PostgresTagReportStore {
    pub store: PostgresStore,
}

impl PostgresTagReportStore {
    async fn query(
        &self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Vec<Row>, TagReportError> {
        let client = self.store.session().await?;
        client
            .query(sql, params)
            .await
            .map_err(|source| TagReportError::Database(backend(source)))
    }

    async fn decoded<Item>(
        &self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Vec<Item>, TagReportError>
    where
        Item: RowDecoder,
    {
        let rows = self.query(sql, params).await?;
        let mut decoded = Vec::with_capacity(rows.len());
        for row in &rows {
            let text: String = row.try_get(0).map_err(backend)?;
            decoded.push(Item::decode_row(&text)?);
        }
        Ok(decoded)
    }
}

#[async_trait]
impl TagReportStore for PostgresTagReportStore {
    async fn keys(&self) -> Result<Vec<LiteLLMVerificationToken>, TagReportError> {
        self.decoded(KEYS_SQL, &[]).await
    }

    async fn keys_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<LiteLLMVerificationToken>, TagReportError> {
        self.decoded(USER_KEYS_SQL, &[&user_id]).await
    }

    async fn users(&self) -> Result<Vec<KeyAliasedUser>, TagReportError> {
        let stored: Vec<UserWithAliases> = self.decoded(USERS_SQL, &[&NO_LIMIT, &NO_LIMIT]).await?;
        Ok(stored.into_iter().map(KeyAliasedUser::from).collect())
    }

    async fn user(&self, user_id: &str) -> Result<Option<LiteLLMUserTable>, TagReportError> {
        let rows: Vec<LiteLLMUserTable> = self.decoded(USER_SQL, &[&user_id]).await?;
        Ok(rows.into_iter().next())
    }

    async fn raw_tag_spend(&self) -> Result<Vec<RawTagSpend>, TagReportError> {
        let rows = self.query(RAW_TAGS_SQL, &[]).await?;
        rows.iter().map(RawTagSpend::try_from).collect()
    }

    async fn tag_names(&self) -> Result<Option<Vec<Option<String>>>, TagReportError> {
        let rows = self.query(TAG_NAMES_SQL, &[]).await?;
        let mut tag_names = Vec::with_capacity(rows.len());
        for row in &rows {
            let tag_name: Option<String> =
                row.try_get("individual_request_tag").map_err(backend)?;
            tag_names.push(tag_name);
        }
        Ok(Some(tag_names))
    }

    async fn daily_tag_spend(
        &self,
        start_date: &str,
        end_date: &str,
        tags: Option<&[String]>,
    ) -> Result<Vec<DailyTagSpend>, TagReportError> {
        match tags {
            None => {
                let rows = self
                    .query(DAILY_ALL_TAGS_SQL, &[&start_date, &end_date])
                    .await?;
                rows.iter().map(DailyTagSpend::try_from).collect()
            }
            Some(tags) => {
                let rows = self
                    .query(DAILY_TAGS_SQL, &[&start_date, &end_date, &tags])
                    .await?;
                rows.iter().map(DailyTagSpend::try_from).collect()
            }
        }
    }
}

impl TryFrom<&Row> for RawTagSpend {
    type Error = TagReportError;

    fn try_from(row: &Row) -> Result<Self, Self::Error> {
        Ok(Self {
            individual_request_tag: row.try_get("individual_request_tag").map_err(backend)?,
            log_count: row.try_get("log_count").map_err(backend)?,
            total_spend: spend_amount(row)?,
        })
    }
}

impl TryFrom<&Row> for DailyTagSpend {
    type Error = TagReportError;

    fn try_from(row: &Row) -> Result<Self, Self::Error> {
        Ok(Self {
            individual_request_tag: row.try_get("individual_request_tag").map_err(backend)?,
            total_spend: spend_amount(row)?,
            log_count: row.try_get("log_count").map_err(backend)?,
        })
    }
}

fn spend_amount(row: &Row) -> Result<Decimal, TagReportError> {
    let stored: Option<String> = row.try_get(SPEND_TEXT_COLUMN).map_err(backend)?;
    let Some(digits) = stored else {
        return Err(TagReportError::NullAmount {
            column: SPEND_TEXT_COLUMN,
        });
    };
    digits
        .parse::<Decimal>()
        .or_else(|_unparsed| Decimal::from_scientific(&digits))
        .map_err(|source| TagReportError::UnreadableAmount {
            column: SPEND_TEXT_COLUMN,
            source,
        })
}
