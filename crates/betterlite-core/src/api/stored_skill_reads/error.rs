use base64::DecodeError;
use betterlite_context::local_ttl_cache::CacheError;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::postgres_store::errors::DbError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoredSkillReadError {
    #[error("Prisma client is not initialized. Database connection required for LiteLLM skills.")]
    NoDatabase,
    #[error("Skill not found: {skill_id}")]
    NotFound { skill_id: String },
    #[error("the skill repository failed")]
    Repository {
        #[source]
        source: Box<TableError>,
    },
    #[error("the skill file content is not valid Base64")]
    FileContent(#[source] DecodeError),
    #[error(transparent)]
    Cache(#[from] CacheError),
}

impl From<TableError> for StoredSkillReadError {
    fn from(error: TableError) -> Self {
        match error {
            TableError::Database(DbError::NoDbConnected) => Self::NoDatabase,
            other => Self::Repository {
                source: Box::new(other),
            },
        }
    }
}
