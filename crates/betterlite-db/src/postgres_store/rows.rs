use super::errors::DbError;
use std::collections::BTreeMap;
use tokio_postgres::Row;

pub type Record = BTreeMap<String, String>;

pub trait RowCodec {
    type Model;
    fn decode(&self, row: &Row) -> Result<Self::Model, DbError>;
}
