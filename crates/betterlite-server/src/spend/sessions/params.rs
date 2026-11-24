use tokio_postgres::types::{ToSql, Type};

// one positional value of a session query; prepare_typed declares the type so the driver never guesses
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionParam {
    Text(String),
    MaybeText(Option<String>),
    Texts(Vec<String>),
    Integer(i64),
}

impl SessionParam {
    #[must_use]
    pub const fn sql_type(&self) -> Type {
        match self {
            Self::Text(_) | Self::MaybeText(_) => Type::TEXT,
            Self::Texts(_) => Type::TEXT_ARRAY,
            Self::Integer(_) => Type::INT8,
        }
    }

    #[must_use]
    pub fn bind(&self) -> &(dyn ToSql + Sync) {
        match self {
            Self::Text(value) => value,
            Self::MaybeText(value) => value,
            Self::Texts(values) => values,
            Self::Integer(value) => value,
        }
    }
}

// the SQL a session read runs and the values it binds, in placeholder order
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionQuery {
    pub sql: String,
    pub params: Vec<SessionParam>,
}
