use crate::identity_rows::key::LiteLLMVerificationToken;
use crate::identity_rows::user::LiteLLMUserTable;
use crate::key_repository::table::TokenValue;
use crate::key_repository::table::columns::TokenColumn;
use crate::key_user_writes::columns::UserColumn;
use crate::table_ops::columns::spend_notification::SpendColumn;
use crate::table_ops::columns::{ConfigColumn, TableColumn};
use crate::table_ops::rows::{ConfigParamRow, RowDecoder, SpendLogRow};

use super::{FilterValue, GenericFilter};

// These four tables are the closed set accepted by get_generic_data.
pub trait GenericColumn: TableColumn {
    type Row: RowDecoder + Send;
}

impl GenericColumn for TokenColumn {
    type Row = LiteLLMVerificationToken;
}

impl GenericColumn for UserColumn {
    type Row = LiteLLMUserTable;
}

impl GenericColumn for ConfigColumn {
    type Row = ConfigParamRow;
}

impl GenericColumn for SpendColumn {
    type Row = SpendLogRow;
}

// All scalar schema columns use the same checked binder as writes, including null predicates.
pub struct ColumnFilter<'value, Column> {
    pub column: Column,
    pub value: &'value TokenValue,
}

impl<'value, Column: GenericColumn> GenericFilter<'value> for ColumnFilter<'value, Column> {
    type Row = Column::Row;
    const ATTRIBUTE: &'static str = Column::ATTRIBUTE;
    const TABLE: &'static str = Column::TABLE;

    fn parts(self) -> (&'static str, FilterValue<'value>) {
        let (name, kind) = self.column.sql_definition();
        (name, FilterValue::Typed(kind, self.value))
    }
}
