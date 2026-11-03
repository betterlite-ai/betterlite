use super::values::JsonMoney;
use deadpool_postgres::GenericClient;
use rust_decimal::Decimal;
use serde::Serialize;
use serde_with::{FromInto, serde_as};
use tokio_postgres::types::{ToSql, Type};

#[serde_as]
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum SqlBind {
    Text(Option<String>),
    Integer(Option<i64>),
    Money(#[serde_as(serialize_as = "FromInto<JsonMoney>")] Decimal),
}

impl SqlBind {
    #[must_use]
    pub fn parameter(&self) -> (&(dyn ToSql + Sync), Type) {
        match self {
            Self::Text(value) => (value, Type::TEXT),
            Self::Integer(value) => (value, Type::INT8),
            Self::Money(value) => (value, Type::NUMERIC),
        }
    }
}

pub async fn execute<C>(
    client: &C,
    sql: &str,
    params: &[SqlBind],
) -> Result<u64, tokio_postgres::Error>
where
    C: GenericClient + Sync,
{
    let (values, types): (Vec<_>, Vec<_>) = params.iter().map(SqlBind::parameter).unzip();
    // Explicit parameter types retain Decimal until PostgreSQL converts to the pinned float columns.
    let statement = client.prepare_typed(sql, &types).await?;
    client.execute(&statement, &values).await
}
