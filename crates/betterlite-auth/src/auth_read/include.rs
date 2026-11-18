use betterlite_db::postgres_store::rows::{
    FindMany, Key, KeyAction, KeyColumn, Query, RowCodec, Table,
};
use tokio_postgres::Row;

// Prisma include maps relation names to flags; only the relations a caller asks for reach the SQL.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UserInclude {
    pub organization_memberships: bool,
}

// The query a caller runs against a table, so a table selects relations and width in one method.
pub enum IncludingQuery<'query, Rows: Table> {
    Keyed {
        action: KeyAction<'query, Rows::Data>,
        key: Key<'query, Rows::Column>,
    },
    Listed {
        filter: Option<&'query Rows::Filter>,
        limit: &'query Option<i64>,
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

// B700's table plus the relation selection and row limit of one request, so the selector sees all three.
pub struct SelectingTable<Rows: Including> {
    pub table: Rows,
    pub include: Option<Rows::Include>,
    pub limit: Option<i64>,
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
        self.table.including(
            IncludingQuery::Listed {
                filter: options.filter.and_then(|filters| filters.first()),
                limit: &self.limit,
            },
            self.include.as_ref(),
        )
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
