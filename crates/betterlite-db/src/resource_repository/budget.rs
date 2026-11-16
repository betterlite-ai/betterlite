use std::convert::Infallible;

use betterlite_redis::config_sync::publish::redis_config_change_publisher;
use betterlite_redis::config_sync::table_actions::ConfigSyncTableActions;
use tokio_postgres::Row;

use crate::auxiliary_repository::repository::{PrismaClient, PrismaTableRepository, TableBinding};
use crate::budget_permission_rows::LiteLLMBudgetTable;
use crate::postgres_store::diagnostics::DriverError;
use crate::postgres_store::errors::DbError;
use crate::postgres_store::repository::Repository;
use crate::postgres_store::rows::{FindMany, Key, KeyAction, KeyColumn, Query, RowCodec, Table};
use crate::resource_rows::{ResourceRowsError, convert, parse_data};

use super::table::{ResourceActions, ResourceBinding, RowWrite};

const BUDGET_BY_ID_SQL: &str = "SELECT row_to_json(entry)::text FROM \"LiteLLM_BudgetTable\" AS entry WHERE entry.\"budget_id\" = $1";
const BUDGET_DELETE_SQL: &str = "DELETE FROM \"LiteLLM_BudgetTable\" AS entry WHERE entry.\"budget_id\" = $1 RETURNING row_to_json(entry)::text";
const BUDGET_UPDATE_SQL: &str = "UPDATE \"LiteLLM_BudgetTable\" AS entry SET \"budget_id\" = \"budget_id\" WHERE entry.\"budget_id\" = $1 RETURNING row_to_json(entry)::text";
const BUDGET_INSERT_SQL: &str = "INSERT INTO \"LiteLLM_BudgetTable\" AS entry DEFAULT VALUES RETURNING row_to_json(entry)::text";
const BUDGET_ALL_SQL: &str =
    "SELECT row_to_json(entry)::text FROM \"LiteLLM_BudgetTable\" AS entry";
const BUDGET_MEMBERSHIP_SQL: &str = "SELECT row_to_json(entry)::text FROM \"LiteLLM_BudgetTable\" AS entry WHERE entry.\"budget_id\" = ANY($1::text[])";
const BUDGET_COUNT_SQL: &str = "SELECT count(*) FROM \"LiteLLM_BudgetTable\"";

pub struct BudgetBinding;

impl TableBinding for BudgetBinding {
    const NAME: &'static str = "litellm_budgettable";
    const TABLE: &'static str = "LiteLLM_BudgetTable";
}

impl ResourceBinding for BudgetBinding {
    const KEY: &'static str = "budget_id";
    const ID: Option<&'static str> = Some("budget_id");
    const UPDATED_AT: Option<&'static str> = Some("updated_at");
    type Create = RowWrite;
    type Update = RowWrite;
}

#[derive(Debug, thiserror::Error)]
pub enum BudgetRowError {
    #[error("stored budget row column could not be read")]
    Column(#[source] DriverError),
    #[error(transparent)]
    Row(#[from] ResourceRowsError),
}

#[derive(Debug, thiserror::Error)]
pub enum BudgetRepositoryError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error(transparent)]
    Query(#[from] DbError<BudgetRowError>),
}

pub(super) struct BudgetTable;

impl RowCodec for BudgetTable {
    type Model = LiteLLMBudgetTable;
    type Error = BudgetRowError;

    fn decode(&self, row: &Row) -> Result<Option<LiteLLMBudgetTable>, BudgetRowError> {
        let stored: String = row
            .try_get(0)
            .map_err(|driver| BudgetRowError::Column(DriverError { driver }))?;
        let record = parse_data(stored.as_bytes())?;
        convert(&record, usize::MAX)
            .map(Some)
            .map_err(BudgetRowError::Row)
    }
}

impl Table for BudgetTable {
    type Column = Infallible;
    type Filter = Infallible;
    type Order = Infallible;
    type Data = ();

    fn keyed<'query>(
        &'query self,
        action: KeyAction<'query, ()>,
        key: Key<'query, Infallible>,
    ) -> Query<'query> {
        match (action, key.column) {
            (KeyAction::Find, KeyColumn::Id) => Query {
                sql: BUDGET_BY_ID_SQL,
                parameters: vec![key.value],
            },
            (KeyAction::Delete, KeyColumn::Id) => Query {
                sql: BUDGET_DELETE_SQL,
                parameters: vec![key.value],
            },
            (KeyAction::Update(()), KeyColumn::Id) => Query {
                sql: BUDGET_UPDATE_SQL,
                parameters: vec![key.value],
            },
            (
                KeyAction::Find | KeyAction::Delete | KeyAction::Update(()),
                KeyColumn::Alternate(never),
            ) => match never {},
        }
    }

    fn list<'query>(
        &'query self,
        _options: FindMany<'query, Infallible, Infallible>,
    ) -> Query<'query> {
        Query {
            sql: BUDGET_ALL_SQL,
            parameters: Vec::new(),
        }
    }

    fn insert<'query>(&'query self, _data: &'query ()) -> Query<'query> {
        Query {
            sql: BUDGET_INSERT_SQL,
            parameters: Vec::new(),
        }
    }

    fn count<'query>(&self, _filter: Option<&'query [Infallible]>) -> Query<'query> {
        Query {
            sql: BUDGET_COUNT_SQL,
            parameters: Vec::new(),
        }
    }

    fn membership(&self, _column: KeyColumn<Infallible>) -> &str {
        BUDGET_MEMBERSHIP_SQL
    }
}

pub struct BudgetRepository<'db> {
    base: PrismaTableRepository<'db, BudgetBinding>,
}

impl<'db> BudgetRepository<'db> {
    #[must_use]
    pub fn new(client: Option<PrismaClient<'db>>) -> Self {
        Self {
            base: PrismaTableRepository::new(
                client,
                redis_config_change_publisher(None),
                BudgetBinding,
            ),
        }
    }

    // ast-grep-ignore: no-one-line-functions - LiteLLM's table property builds the same actions
    pub fn table(
        &self,
    ) -> Result<ConfigSyncTableActions<ResourceActions<'db, BudgetBinding>>, DbError> {
        self.base.table(ResourceActions::new)
    }

    pub async fn find_by_id(
        &self,
        budget_id: &str,
    ) -> Result<Option<LiteLLMBudgetTable>, BudgetRepositoryError> {
        let access = self.base.access()?;
        let client = access.read.session().await?;
        let key = Key {
            column: KeyColumn::Id,
            value: &budget_id,
        };
        Repository {
            client: Some(&client),
            table: BudgetTable,
        }
        .by_key(KeyAction::Find, key)
        .await
        .map_err(BudgetRepositoryError::from)
    }
}
