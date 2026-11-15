use betterlite_db::auxiliary_repository::repository::TableError;

#[derive(Debug, thiserror::Error)]
pub enum ManagedFileLookupError {
    #[error("the managed-file row lookup failed")]
    Lookup(#[source] TableError),
}
