pub mod clock;
pub mod records;
pub mod store;

use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use betterlite_db::postgres_store::errors::DbError;
use deadpool_postgres::GenericClient;

use crate::audit_events::identity::CallerIdentity;

use self::clock::ArchiveClock;

#[derive(Debug, thiserror::Error)]
pub enum KeyArchiveError {
    #[error("the deleted key archive write failed")]
    Database {
        #[source]
        source: DbError,
    },
    #[error("the archived key's {column} value is not a timestamp")]
    InvalidTimestamp { column: &'static str },
    #[error("the archived key's {column} value is not text")]
    NonTextValue { column: &'static str },
}

// ast-grep-ignore: no-long-names
pub async fn persist_deleted_verification_tokens<C>(
    db: &C,
    keys: &[LiteLLMVerificationToken],
    caller: &CallerIdentity,
    litellm_changed_by: Option<&str>,
    clock: &dyn ArchiveClock,
) -> Result<(), KeyArchiveError>
where
    C: GenericClient + Sync,
{
    // ast-grep-ignore: no-long-names
    let records = records::transform_verification_tokens_to_deleted_records(
        keys,
        caller,
        litellm_changed_by,
        clock,
    );
    // ast-grep-ignore: no-long-names
    store::save_deleted_verification_token_records(db, &records).await
}
