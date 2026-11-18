#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableName {
    User,
    Key,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum QueryType {
    #[default]
    Update,
    UpdateMany,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateTarget {
    Key,
    User,
    Elsewhere,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct UpdateSelector<'selector> {
    pub token: Option<&'selector str>,
    pub user_id: Option<&'selector str>,
    pub table_name: Option<TableName>,
    pub query_type: QueryType,
}

impl UpdateSelector<'_> {
    // a token wins even when empty, and a supplied user_id wins over update_many, exactly as the source elif chain does
    #[must_use]
    pub const fn target(self) -> UpdateTarget {
        if self.token.is_some() {
            return UpdateTarget::Key;
        }
        if self.user_id.is_some() {
            return UpdateTarget::User;
        }
        match (self.table_name, self.query_type) {
            (Some(TableName::User), QueryType::Update) => UpdateTarget::User,
            _ => UpdateTarget::Elsewhere,
        }
    }
}
