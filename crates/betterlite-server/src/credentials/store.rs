use async_trait::async_trait;
use betterlite_db::resource_repository::credential::CredentialError as RepositoryError;
use betterlite_db::resource_rows::credential::CredentialItem;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CredentialStoreError {
    #[error("a credential with this name already exists")]
    Duplicate,
    #[error("credential storage failed")]
    Repository {
        #[source]
        source: RepositoryError,
    },
}

#[async_trait]
pub trait CredentialStore: Send + Sync {
    async fn create(
        &self,
        credential: &CredentialItem,
        actor: Option<&str>,
    ) -> Result<(), CredentialStoreError>;

    async fn find_by_name(
        &self,
        credential_name: &str,
    ) -> Result<Option<CredentialItem>, CredentialStoreError>;

    async fn update(
        &self,
        credential_name: &str,
        credential: &CredentialItem,
        actor: Option<&str>,
    ) -> Result<(), CredentialStoreError>;

    // true when a row was deleted
    async fn delete(&self, credential_name: &str) -> Result<bool, CredentialStoreError>;
}
