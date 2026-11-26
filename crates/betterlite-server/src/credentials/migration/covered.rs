use std::error::Error;
use std::future::Future;
use std::sync::Arc;

use betterlite_db::encryption_scan::DbCipher;
use betterlite_db::sso_assertion_rows::AssertionRows;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use secrecy::SecretString;
use tokio_util::task::TaskTracker;

use crate::config_override_support::environment::RuntimeEnvironment;
use crate::credentials::rotation::Rotation;
use crate::credentials::rotation::error::RotationError;
use crate::credentials::rotation::sink::ModelTableInvalidation;
use crate::credentials::rotation::store::RotationStore;
use crate::db_row_decrypt::warnings::Warnings;
use crate::mcp_rotation::store::McpRotationStore;

// _migrate_covered_tables runs _rotate_master_key with the configured key as both the current and the new key.
pub trait SameKeyRotation {
    type Error: Error + Send + 'static;

    fn reencrypt_same_key(
        &mut self,
        key: &SecretString,
        actor: Option<&str>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

// B1700's rotation with the caller's stores; the migration supplies the key it read after the pre-scan.
pub struct StoredRotation<'run, S, M, A, I> {
    pub store: &'run S,
    pub invalidate: &'run I,
    pub mcp: &'run M,
    pub assertions: &'run A,
    pub cipher: &'run Arc<DbCipher>,
    pub secrets: &'run Arc<SecretLookup>,
    pub warnings: &'run Arc<Warnings>,
    pub tasks: &'run TaskTracker,
    pub runtime: &'run mut RuntimeEnvironment,
}

impl<S, M, A, I> SameKeyRotation for StoredRotation<'_, S, M, A, I>
where
    S: RotationStore,
    M: McpRotationStore,
    A: AssertionRows + Sync,
    I: ModelTableInvalidation,
{
    type Error = RotationError<S::Error, I::Error>;

    // One call into B1700's rotation; exercising it needs that task's fixture stores, which this module cannot reach.
    #[cfg_attr(test, mutants::skip)]
    async fn reencrypt_same_key(
        &mut self,
        key: &SecretString,
        actor: Option<&str>,
    ) -> Result<(), RotationError<S::Error, I::Error>> {
        let mut rotation = Rotation {
            store: self.store,
            invalidate: self.invalidate,
            mcp: self.mcp,
            assertions: self.assertions,
            cipher: self.cipher,
            new_master_key: key,
            secrets: self.secrets,
            warnings: self.warnings,
            tasks: self.tasks,
            runtime: &mut *self.runtime,
        };
        rotation.run(actor).await
    }
}
