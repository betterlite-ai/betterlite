use std::convert::Infallible;

use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::postgres_store::rows::{
    FindMany, Key, KeyAction, KeyColumn, Query, RowCodec, Table,
};
use betterlite_db::user_repository::tables::StoredTable;
use tokio_postgres::Row;

const USER_BY_ID_WITH_MEMBERSHIPS_SQL: &str = "SELECT (to_jsonb(entry) || jsonb_build_object('organization_memberships', COALESCE((SELECT jsonb_agg(member) FROM \"LiteLLM_OrganizationMembership\" AS member WHERE member.\"user_id\" = entry.\"user_id\"), '[]'::jsonb)))::text FROM \"LiteLLM_UserTable\" AS entry WHERE \"user_id\" = $1";

const USER_ALL_WITH_MEMBERSHIPS_SQL: &str = "SELECT (to_jsonb(entry) || jsonb_build_object('organization_memberships', COALESCE((SELECT jsonb_agg(member) FROM \"LiteLLM_OrganizationMembership\" AS member WHERE member.\"user_id\" = entry.\"user_id\"), '[]'::jsonb)))::text FROM \"LiteLLM_UserTable\" AS entry";

const USER_INSERT_WITH_MEMBERSHIPS_SQL: &str = "INSERT INTO \"LiteLLM_UserTable\" AS entry DEFAULT VALUES RETURNING (to_jsonb(entry) || jsonb_build_object('organization_memberships', COALESCE((SELECT jsonb_agg(member) FROM \"LiteLLM_OrganizationMembership\" AS member WHERE member.\"user_id\" = entry.\"user_id\"), '[]'::jsonb)))::text";

// Prisma include maps relation names to flags; only the relations a caller asks for reach the SQL.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UserInclude {
    pub organization_memberships: bool,
}

// The query a caller runs against a table, so a table selects relations in one method.
pub enum IncludingQuery<'query, Rows: Table> {
    Keyed {
        action: KeyAction<'query, Rows::Data>,
        key: Key<'query, Rows::Column>,
    },
    Listed {
        options: FindMany<'query, Rows::Filter, Rows::Order>,
    },
    Inserted {
        data: &'query Rows::Data,
    },
}

// A table whose row query can select relations as well as its own columns.
pub trait Including: Table + Sized {
    type Include: Clone + Send + Sync + 'static;

    fn including<'query>(
        &'query self,
        query: IncludingQuery<'query, Self>,
        include: Option<&'query Self::Include>,
    ) -> Query<'query>;
}

// B700's table plus the relation selection of one request, so the query selector sees both.
pub struct SelectingTable<Rows: Including> {
    pub table: Rows,
    pub include: Option<Rows::Include>,
}

impl<Rows: Including> RowCodec for SelectingTable<Rows> {
    type Model = Rows::Model;
    type Error = Rows::Error;

    fn decode(&self, row: &Row) -> Result<Option<Self::Model>, Self::Error> {
        self.table.decode(row)
    }
}

impl<Rows: Including> Table for SelectingTable<Rows> {
    type Column = Rows::Column;
    type Filter = Rows::Filter;
    type Order = Rows::Order;
    type Data = Rows::Data;

    fn keyed<'query>(
        &'query self,
        action: KeyAction<'query, Self::Data>,
        key: Key<'query, Self::Column>,
    ) -> Query<'query> {
        self.table
            .including(IncludingQuery::Keyed { action, key }, self.include.as_ref())
    }

    fn list<'query>(
        &'query self,
        options: FindMany<'query, Self::Filter, Self::Order>,
    ) -> Query<'query> {
        self.table
            .including(IncludingQuery::Listed { options }, self.include.as_ref())
    }

    fn insert<'query>(&'query self, data: &'query Self::Data) -> Query<'query> {
        self.table
            .including(IncludingQuery::Inserted { data }, self.include.as_ref())
    }

    fn count<'query>(&'query self, filter: Option<&'query [Self::Filter]>) -> Query<'query> {
        self.table.count(filter)
    }

    fn membership(&self, column: KeyColumn<Self::Column>) -> &str {
        self.table.membership(column)
    }
}

impl Including for StoredTable<LiteLLMUserTable> {
    type Include = UserInclude;

    fn including<'query>(
        &'query self,
        query: IncludingQuery<'query, Self>,
        include: Option<&'query Self::Include>,
    ) -> Query<'query> {
        let memberships = include.is_some_and(|include| include.organization_memberships);
        match query {
            IncludingQuery::Keyed { action, key } => match (action, key.column, memberships) {
                (KeyAction::Find, KeyColumn::Id, true) => Query {
                    sql: USER_BY_ID_WITH_MEMBERSHIPS_SQL,
                    parameters: vec![key.value],
                },
                (action, column, _) => Table::keyed(
                    self,
                    action,
                    Key {
                        column,
                        value: key.value,
                    },
                ),
            },
            IncludingQuery::Listed { .. } if memberships => Query {
                sql: USER_ALL_WITH_MEMBERSHIPS_SQL,
                parameters: Vec::new(),
            },
            IncludingQuery::Listed { options } => Table::list(self, options),
            IncludingQuery::Inserted { .. } if memberships => Query {
                sql: USER_INSERT_WITH_MEMBERSHIPS_SQL,
                parameters: Vec::new(),
            },
            IncludingQuery::Inserted { data } => Table::insert(self, data),
        }
    }
}
