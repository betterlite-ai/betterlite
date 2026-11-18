use crate::table_ops::access::ReadQuery;
use crate::table_ops::access::TableAccess as _;
use secrecy::{ExposeSecret as _, SecretString};
use std::future::Future;
use tokio_postgres::types::ToSql;

use crate::budget_permission_rows::Timestamp;
use crate::database_replica::client::DatabaseClient;
use crate::identity_rows::key::LiteLLMVerificationToken;
use crate::identity_rows::user::LiteLLMUserTable;

use super::access::{
    CONFIG_ATTRIBUTE, CONFIG_TABLE, KEY_ATTRIBUTE, KEY_TABLE, SPEND_ATTRIBUTE, SPEND_TABLE,
    TablePoolAccess, USER_ATTRIBUTE, USER_TABLE,
};
use super::reads::spend::SpendFilterColumn;
use super::reads::users::UserFilter;
use super::rows::{ConfigParamRow, RowDecoder, RowError, SpendLogRow};
use super::{GENERIC_RETRY, OperationCallType, TableOps, TableOpsError};

// LiteLLM's find_first filters on any column of the table it reads, not only the primary key
#[derive(Debug, Clone, Copy)]
pub enum KeyFilter<'value> {
    Token(&'value SecretString),
    UserId(&'value str),
    TeamId(&'value str),
    KeyAlias(&'value str),
    Blocked(bool),
}

#[derive(Debug, Clone, Copy)]
pub enum ConfigFilter<'value> {
    ParamName(&'value str),
    LastRunAt(&'value Timestamp),
    ReloadRevision(i64),
}

#[derive(Debug, Clone, Copy)]
pub enum FilterValue<'value> {
    Text(&'value str),
    Timestamp(&'value Timestamp),
    Integer(i64),
    Boolean(bool),
}

// LiteLLM picks the table and key column at runtime; each filter type names both, and its row type
pub trait GenericFilter<'value> {
    type Row: RowDecoder + Send;
    const ATTRIBUTE: &'static str;
    const TABLE: &'static str;
    fn parts(self) -> (&'static str, FilterValue<'value>);
}

impl<'value> GenericFilter<'value> for UserFilter<'value> {
    type Row = LiteLLMUserTable;
    const ATTRIBUTE: &'static str = USER_ATTRIBUTE;
    const TABLE: &'static str = USER_TABLE;

    fn parts(self) -> (&'static str, FilterValue<'value>) {
        let (column, value) = self.column();
        (column, FilterValue::Text(value))
    }
}

impl<'value> GenericFilter<'value> for KeyFilter<'value> {
    type Row = LiteLLMVerificationToken;
    const ATTRIBUTE: &'static str = KEY_ATTRIBUTE;
    const TABLE: &'static str = KEY_TABLE;

    fn parts(self) -> (&'static str, FilterValue<'value>) {
        match self {
            Self::Token(token) => ("token", FilterValue::Text(token.expose_secret())),
            Self::UserId(user_id) => ("user_id", FilterValue::Text(user_id)),
            Self::TeamId(team_id) => ("team_id", FilterValue::Text(team_id)),
            Self::KeyAlias(alias) => ("key_alias", FilterValue::Text(alias)),
            Self::Blocked(flag) => ("blocked", FilterValue::Boolean(flag)),
        }
    }
}

impl<'value> GenericFilter<'value> for ConfigFilter<'value> {
    type Row = ConfigParamRow;
    const ATTRIBUTE: &'static str = CONFIG_ATTRIBUTE;
    const TABLE: &'static str = CONFIG_TABLE;

    fn parts(self) -> (&'static str, FilterValue<'value>) {
        match self {
            Self::ParamName(name) => ("param_name", FilterValue::Text(name)),
            Self::LastRunAt(timestamp) => ("last_run_at", FilterValue::Timestamp(timestamp)),
            Self::ReloadRevision(revision) => ("reload_revision", FilterValue::Integer(revision)),
        }
    }
}

pub trait GenericReads<Client>
where
    Client: DatabaseClient,
    Client::Attribute: TablePoolAccess,
{
    fn generic_row<'value, Filter>(
        &self,
        filter: Filter,
    ) -> impl Future<Output = Result<Option<Filter::Row>, TableOpsError<Client::Error>>> + Send
    where
        Filter: GenericFilter<'value> + Send;
    fn generic_spend(
        &self,
        column: SpendFilterColumn,
        value: &str,
    ) -> impl Future<Output = Result<Option<SpendLogRow>, TableOpsError<Client::Error>>> + Send;
    fn find_generic<Row, Parse>(
        &self,
        attribute: &str,
        sql: &str,
        value: &(dyn ToSql + Sync),
        parse: Parse,
    ) -> impl Future<Output = Result<Option<Row>, TableOpsError<Client::Error>>> + Send
    where
        Row: Send,
        Parse: Fn(&str) -> Result<Row, RowError> + Send + Sync;
}

impl<Client> GenericReads<Client> for TableOps<'_, Client>
where
    Client: DatabaseClient,
    Client::Attribute: TablePoolAccess,
{
    async fn generic_row<'value, Filter>(
        &self,
        filter: Filter,
    ) -> Result<Option<Filter::Row>, TableOpsError<Client::Error>>
    where
        Filter: GenericFilter<'value> + Send,
    {
        let (column, value) = filter.parts();
        let sql = format!(
            "SELECT row_to_json(t)::text FROM \"{}\" AS t WHERE t.\"{column}\" = $1 LIMIT 1",
            Filter::TABLE,
        );
        let moment;
        let bound: &(dyn ToSql + Sync) = match &value {
            FilterValue::Text(text) => text,
            FilterValue::Integer(number) => number,
            FilterValue::Boolean(flag) => flag,
            FilterValue::Timestamp(timestamp) => {
                moment = timestamp.utc().naive_utc();
                &moment
            }
        };
        self.find_generic(Filter::ATTRIBUTE, &sql, bound, Filter::Row::decode_row)
            .await
    }

    async fn generic_spend(
        &self,
        column: SpendFilterColumn,
        value: &str,
    ) -> Result<Option<SpendLogRow>, TableOpsError<Client::Error>> {
        let sql = format!(
            "SELECT row_to_json(t)::text FROM \"{}\" AS t WHERE t.\"{}\" = $1 LIMIT 1",
            SPEND_TABLE,
            column.name(),
        );
        self.find_generic(SPEND_ATTRIBUTE, &sql, &value, SpendLogRow::decode_row)
            .await
    }

    async fn find_generic<Row, Parse>(
        &self,
        attribute: &str,
        sql: &str,
        value: &(dyn ToSql + Sync),
        parse: Parse,
    ) -> Result<Option<Row>, TableOpsError<Client::Error>>
    where
        Row: Send,
        Parse: Fn(&str) -> Result<Row, RowError> + Send + Sync,
    {
        let rows = self
            .query_rows(
                ReadQuery {
                    attribute,
                    method: "find_first",
                    sql,
                    parameters: &[value],
                },
                GENERIC_RETRY,
                OperationCallType::GetGenericData,
                parse,
            )
            .await?;
        Ok(rows.into_iter().next())
    }
}
