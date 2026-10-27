use super::errors::DbError;
use tokio_postgres::{Row, types::ToSql};

pub trait RowCodec {
    type Model;
    fn decode(&self, row: &Row) -> Result<Option<Self::Model>, DbError>;
}

pub struct Query<'query> {
    pub sql: &'query str,
    pub parameters: Vec<&'query (dyn ToSql + Sync)>,
}

#[derive(Clone, Copy, Default)]
pub enum KeyColumn<C> {
    #[default]
    Id,
    Alternate(C),
}

pub struct Key<'query, C> {
    pub column: KeyColumn<C>,
    pub value: &'query (dyn ToSql + Sync),
}

#[derive(Clone, Copy)]
pub enum KeyAction {
    Find,
    Update,
    Delete,
}

#[derive(Default)]
pub struct FindMany<'query, F, O> {
    pub filter: Option<&'query [F]>,
    pub order: Option<&'query [O]>,
    pub skip: Option<i64>,
    pub take: Option<i64>,
}

pub trait Table: RowCodec {
    type Column;
    type Filter;
    type Order;
    type Data;
    fn keyed<'query>(
        &'query self,
        action: KeyAction,
        key: Key<'query, Self::Column>,
        data: Option<&'query Self::Data>,
    ) -> Query<'query>;
    fn list<'query>(
        &'query self,
        options: FindMany<'query, Self::Filter, Self::Order>,
    ) -> Query<'query>;
    fn insert<'query>(&'query self, data: &'query Self::Data) -> Query<'query>;
    fn count<'query>(&'query self, filter: Option<&'query [Self::Filter]>) -> Query<'query>;
    fn membership(&self, column: KeyColumn<Self::Column>) -> &str;
}
