use std::sync::Arc;

use betterlite_db::database_failures::gate::LookupClock;

use super::table::{AuthReadTable, AuthTable, BoundedTable, ReadContext};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthReadAdapter {
    Dictable,
    ModelDump,
    JwtKeyMapping,
    Team,
    VectorStore,
    User,
    ObjectPermission,
    Tag,
    EndUser,
    ModelAccessGroupBudget,
}

impl AuthReadAdapter {
    #[must_use]
    pub fn open<T, Clock>(
        self,
        table: Arc<T>,
        caller_label: String,
        context: ReadContext<Clock>,
    ) -> AuthReadTable<T, Clock>
    where
        T: AuthTable,
        Clock: LookupClock + 'static,
    {
        let label = match self {
            Self::Dictable | Self::ModelDump => caller_label,
            Self::JwtKeyMapping => "jwt_key_mapping".to_owned(),
            Self::Team => "team".to_owned(),
            Self::VectorStore => "vector_store".to_owned(),
            Self::User => "user".to_owned(),
            Self::ObjectPermission => "object_permission".to_owned(),
            Self::Tag | Self::EndUser | Self::ModelAccessGroupBudget => {
                return AuthReadTable::Unbounded(table);
            }
        };
        AuthReadTable::Bounded(Box::new(BoundedTable {
            table,
            lookup: label,
            context,
        }))
    }
}
