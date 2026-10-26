use chrono::NaiveDateTime;
use rust_decimal::Decimal;

use super::schema::ColumnType;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ColumnValues {
    Decimal(Vec<Option<Decimal>>),
    Text(Vec<Option<String>>),
    Timestamp(Vec<Option<NaiveDateTime>>),
}

impl ColumnValues {
    #[must_use]
    pub const fn empty(kind: ColumnType) -> Self {
        match kind {
            ColumnType::Decimal => Self::Decimal(Vec::new()),
            ColumnType::Text => Self::Text(Vec::new()),
            ColumnType::Timestamp => Self::Timestamp(Vec::new()),
        }
    }

    #[must_use]
    pub const fn kind(&self) -> ColumnType {
        match self {
            Self::Decimal(_) => ColumnType::Decimal,
            Self::Text(_) => ColumnType::Text,
            Self::Timestamp(_) => ColumnType::Timestamp,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FocusColumn {
    pub name: &'static str,
    pub values: ColumnValues,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FocusTable {
    pub rows: usize,
    pub columns: Vec<FocusColumn>,
}
