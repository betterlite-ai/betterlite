use betterlite_db::postgres_store::errors::backend;
use rust_decimal::Decimal;
use tokio_postgres::Row;
use tokio_postgres::types::FromSql;

use super::error::AnalyticsError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActiveUsersReport {
    Daily,
    Weekly,
    Monthly,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DistinctTagRow {
    pub tag: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActiveUsersRow {
    pub tag: String,
    pub active_users: i64,
    pub date: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TagSummaryRow {
    pub tag: String,
    pub unique_users: i64,
    pub total_requests: Option<i64>,
    pub successful_requests: Option<i64>,
    pub failed_requests: Option<i64>,
    pub total_tokens: Option<i64>,
    pub total_spend: Option<Decimal>,
}

// no Debug: the row carries the caller's api_key, and Debug would print it into logs and errors
#[derive(Clone, Eq, PartialEq)]
pub struct TagSpendRecord {
    pub tag: Option<String>,
    pub api_key: String,
    pub api_requests: Option<i64>,
    pub successful_requests: Option<i64>,
    pub failed_requests: Option<i64>,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub spend: Option<Decimal>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenOwnerRow {
    pub token: String,
    pub user_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserEmailRow {
    pub user_id: String,
    pub user_email: Option<String>,
}

fn column<T>(row: &Row, name: &'static str) -> Result<T, AnalyticsError>
where
    T: for<'row> FromSql<'row>,
{
    row.try_get(name)
        .map_err(|source| AnalyticsError::Database(backend(source)))
}

// every row type reads its columns by field name, so each decode is one field left to right
macro_rules! decode_columns {
    ($row:ident => $record:ident { $($field:ident),+ $(,)? }) => {
        impl TryFrom<&Row> for $record {
            type Error = AnalyticsError;

            fn try_from($row: &Row) -> Result<Self, Self::Error> {
                Ok(Self {
                    $($field: column($row, stringify!($field))?,)+
                })
            }
        }
    };
}

decode_columns!(row => DistinctTagRow { tag });
decode_columns!(row => ActiveUsersRow { tag, active_users, date });
decode_columns!(row => TagSummaryRow {
    tag,
    unique_users,
    total_requests,
    successful_requests,
    failed_requests,
    total_tokens,
    total_spend,
});
decode_columns!(row => TagSpendRecord {
    tag,
    api_key,
    api_requests,
    successful_requests,
    failed_requests,
    prompt_tokens,
    completion_tokens,
    spend,
});
decode_columns!(row => TokenOwnerRow { token, user_id });
decode_columns!(row => UserEmailRow { user_id, user_email });
