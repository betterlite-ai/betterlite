use std::convert::Infallible;

use serde::de::DeserializeOwned;
use tokio_postgres::Row;

use super::RowDecodeError;
use super::projection::{MAX_USER_JSON_DEPTH, decode_user_record};
use crate::identity_rows::user::LiteLLMUserTable;
use crate::postgres_store::diagnostics::DriverError;
use crate::postgres_store::rows::{FindMany, Key, KeyAction, KeyColumn, Query, RowCodec, Table};
use crate::resource_rows::organization::OrganizationTable;
use crate::resource_rows::project::ProjectTable;
use crate::resource_rows::{convert, parse_data};

const USER_BY_ID_SQL: &str =
    "SELECT row_to_json(entry)::text FROM \"LiteLLM_UserTable\" AS entry WHERE \"user_id\" = $1";
const USER_DELETE_SQL: &str = "DELETE FROM \"LiteLLM_UserTable\" AS entry WHERE \"user_id\" = $1 RETURNING row_to_json(entry)::text";
const USER_ALL_SQL: &str = "SELECT row_to_json(entry)::text FROM \"LiteLLM_UserTable\" AS entry";
const USER_MEMBERSHIP_SQL: &str = "SELECT row_to_json(entry)::text FROM \"LiteLLM_UserTable\" AS entry WHERE entry.\"user_email\" = ANY($1::text[])";
const USER_COUNT_SQL: &str = "SELECT count(*) FROM \"LiteLLM_UserTable\"";
const USER_EMAIL_PAGE_SQL: &str = "SELECT row_to_json(entry)::text FROM \"LiteLLM_UserTable\" AS entry WHERE LOWER(entry.\"user_email\") IN (SELECT LOWER(candidate) FROM unnest($1::text[]) AS candidate)";
const USER_SCIM_DEACTIVATED_SQL: &str = "SELECT count(*) FROM \"LiteLLM_UserTable\" WHERE \"metadata\" -> 'scim_active' = 'false'::jsonb";
// no caller builds a user create or update payload here; these statements keep the shared table contract total
const USER_INSERT_SQL: &str =
    "INSERT INTO \"LiteLLM_UserTable\" AS entry DEFAULT VALUES RETURNING row_to_json(entry)::text";
const USER_UPDATE_SQL: &str = "UPDATE \"LiteLLM_UserTable\" AS entry SET \"user_id\" = \"user_id\" WHERE \"user_id\" = $1 RETURNING row_to_json(entry)::text";

const ORGANIZATION_BY_ID_SQL: &str = "SELECT row_to_json(entry)::text FROM \"LiteLLM_OrganizationTable\" AS entry WHERE \"organization_id\" = $1";
const ORGANIZATION_DELETE_SQL: &str = "DELETE FROM \"LiteLLM_OrganizationTable\" AS entry WHERE \"organization_id\" = $1 RETURNING row_to_json(entry)::text";
const ORGANIZATION_ALL_SQL: &str =
    "SELECT row_to_json(entry)::text FROM \"LiteLLM_OrganizationTable\" AS entry";
const ORGANIZATION_MEMBERSHIP_SQL: &str = "SELECT row_to_json(entry)::text FROM \"LiteLLM_OrganizationTable\" AS entry WHERE entry.\"organization_id\" = ANY($1::text[])";
const ORGANIZATION_COUNT_SQL: &str = "SELECT count(*) FROM \"LiteLLM_OrganizationTable\"";
// no caller builds an organization create or update payload here; these statements keep the shared table contract total
const ORGANIZATION_INSERT_SQL: &str = "INSERT INTO \"LiteLLM_OrganizationTable\" AS entry DEFAULT VALUES RETURNING row_to_json(entry)::text";
const ORGANIZATION_UPDATE_SQL: &str = "UPDATE \"LiteLLM_OrganizationTable\" AS entry SET \"organization_id\" = \"organization_id\" WHERE \"organization_id\" = $1 RETURNING row_to_json(entry)::text";

const PROJECT_BY_ID_SQL: &str = "SELECT row_to_json(entry)::text FROM \"LiteLLM_ProjectTable\" AS entry WHERE \"project_id\" = $1";
const PROJECT_DELETE_SQL: &str = "DELETE FROM \"LiteLLM_ProjectTable\" AS entry WHERE \"project_id\" = $1 RETURNING row_to_json(entry)::text";
const PROJECT_ALL_SQL: &str =
    "SELECT row_to_json(entry)::text FROM \"LiteLLM_ProjectTable\" AS entry";
const PROJECT_MEMBERSHIP_SQL: &str = "SELECT row_to_json(entry)::text FROM \"LiteLLM_ProjectTable\" AS entry WHERE entry.\"project_id\" = ANY($1::text[])";
const PROJECT_COUNT_SQL: &str = "SELECT count(*) FROM \"LiteLLM_ProjectTable\"";
// no caller builds a project create or update payload here; these statements keep the shared table contract total
const PROJECT_INSERT_SQL: &str = "INSERT INTO \"LiteLLM_ProjectTable\" AS entry DEFAULT VALUES RETURNING row_to_json(entry)::text";
const PROJECT_UPDATE_SQL: &str = "UPDATE \"LiteLLM_ProjectTable\" AS entry SET \"project_id\" = \"project_id\" WHERE \"project_id\" = $1 RETURNING row_to_json(entry)::text";

pub struct StoredTable<M> {
    by_id_sql: &'static str,
    delete_by_id_sql: &'static str,
    update_by_id_sql: &'static str,
    insert_sql: &'static str,
    select_all_sql: &'static str,
    membership_sql: &'static str,
    count_all_sql: &'static str,
    filtered_sql: &'static str,
    filtered_count_sql: &'static str,
    decode_row: fn(&Row) -> Result<Option<M>, RowDecodeError>,
}

impl<M> Clone for StoredTable<M> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<M> Copy for StoredTable<M> {}

pub enum UserFilter {
    EmailPage(Vec<String>),
    ScimDeactivated,
}

pub const USER_TABLE: StoredTable<LiteLLMUserTable> = StoredTable {
    by_id_sql: USER_BY_ID_SQL,
    delete_by_id_sql: USER_DELETE_SQL,
    update_by_id_sql: USER_UPDATE_SQL,
    insert_sql: USER_INSERT_SQL,
    select_all_sql: USER_ALL_SQL,
    membership_sql: USER_MEMBERSHIP_SQL,
    count_all_sql: USER_COUNT_SQL,
    filtered_sql: USER_EMAIL_PAGE_SQL,
    filtered_count_sql: USER_SCIM_DEACTIVATED_SQL,
    decode_row: decode_user_row,
};

pub const ORGANIZATION_TABLE: StoredTable<OrganizationTable> = StoredTable {
    by_id_sql: ORGANIZATION_BY_ID_SQL,
    delete_by_id_sql: ORGANIZATION_DELETE_SQL,
    update_by_id_sql: ORGANIZATION_UPDATE_SQL,
    insert_sql: ORGANIZATION_INSERT_SQL,
    select_all_sql: ORGANIZATION_ALL_SQL,
    membership_sql: ORGANIZATION_MEMBERSHIP_SQL,
    count_all_sql: ORGANIZATION_COUNT_SQL,
    filtered_sql: ORGANIZATION_ALL_SQL,
    filtered_count_sql: ORGANIZATION_COUNT_SQL,
    decode_row: decode_stored_row::<OrganizationTable>,
};

pub const PROJECT_TABLE: StoredTable<ProjectTable> = StoredTable {
    by_id_sql: PROJECT_BY_ID_SQL,
    delete_by_id_sql: PROJECT_DELETE_SQL,
    update_by_id_sql: PROJECT_UPDATE_SQL,
    insert_sql: PROJECT_INSERT_SQL,
    select_all_sql: PROJECT_ALL_SQL,
    membership_sql: PROJECT_MEMBERSHIP_SQL,
    count_all_sql: PROJECT_COUNT_SQL,
    filtered_sql: PROJECT_ALL_SQL,
    filtered_count_sql: PROJECT_COUNT_SQL,
    decode_row: decode_stored_row::<ProjectTable>,
};

impl<M> RowCodec for StoredTable<M> {
    type Model = M;
    type Error = RowDecodeError;

    fn decode(&self, row: &Row) -> Result<Option<M>, RowDecodeError> {
        (self.decode_row)(row)
    }
}

impl<M> Table for StoredTable<M> {
    type Column = Infallible;
    type Filter = UserFilter;
    type Order = Infallible;
    type Data = ();

    fn keyed<'query>(
        &'query self,
        action: KeyAction<'query, ()>,
        key: Key<'query, Infallible>,
    ) -> Query<'query> {
        match (action, key.column) {
            (KeyAction::Find, KeyColumn::Id) => Query {
                sql: self.by_id_sql,
                parameters: vec![key.value],
            },
            (KeyAction::Delete, KeyColumn::Id) => Query {
                sql: self.delete_by_id_sql,
                parameters: vec![key.value],
            },
            (KeyAction::Update(()), KeyColumn::Id) => Query {
                sql: self.update_by_id_sql,
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
        options: FindMany<'query, UserFilter, Infallible>,
    ) -> Query<'query> {
        match options.filter {
            Some([UserFilter::EmailPage(values)]) => page_query(self, values),
            // the SCIM filter only feeds count(); a row list of it has no caller
            _ => all_rows_query(self),
        }
    }

    fn insert<'query>(&'query self, _data: &'query ()) -> Query<'query> {
        Query {
            sql: self.insert_sql,
            parameters: Vec::new(),
        }
    }

    fn count<'query>(&'query self, filter: Option<&'query [UserFilter]>) -> Query<'query> {
        match filter {
            Some([UserFilter::ScimDeactivated]) => deactivated_count_query(self),
            // email pages only feed find_many(); a count of them has no caller
            _ => total_count_query(self),
        }
    }

    fn membership(&self, _column: KeyColumn<Infallible>) -> &str {
        self.membership_sql
    }
}

fn page_query<'query, M>(
    table: &'query StoredTable<M>,
    values: &'query Vec<String>,
) -> Query<'query> {
    Query {
        sql: table.filtered_sql,
        parameters: vec![values],
    }
}

fn all_rows_query<M>(table: &StoredTable<M>) -> Query<'_> {
    Query {
        sql: table.select_all_sql,
        parameters: Vec::new(),
    }
}

fn deactivated_count_query<M>(table: &StoredTable<M>) -> Query<'_> {
    Query {
        sql: table.filtered_count_sql,
        parameters: Vec::new(),
    }
}

fn total_count_query<M>(table: &StoredTable<M>) -> Query<'_> {
    Query {
        sql: table.count_all_sql,
        parameters: Vec::new(),
    }
}

fn decode_user_row(row: &Row) -> Result<Option<LiteLLMUserTable>, RowDecodeError> {
    let record: String = row.try_get(0).map_err(|source| RowDecodeError::Column {
        source: DriverError { driver: source },
    })?;
    decode_user_record(record.as_bytes()).map_err(|source| RowDecodeError::User { source })
}

fn decode_stored_row<M>(row: &Row) -> Result<Option<M>, RowDecodeError>
where
    M: DeserializeOwned,
{
    let stored: String = row.try_get(0).map_err(|source| RowDecodeError::Column {
        source: DriverError { driver: source },
    })?;
    let record = parse_data(stored.as_bytes()).map_err(|source| RowDecodeError::Row { source })?;
    convert(&record, MAX_USER_JSON_DEPTH)
        .map(Some)
        .map_err(|source| RowDecodeError::Row { source })
}
