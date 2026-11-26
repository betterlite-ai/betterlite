use betterlite_db::encryption_scan::DbCipher;
use betterlite_db::encryption_scan::report::{LocationReport, MigrationReport};
use betterlite_secrets::secret_codec::Algorithm;

use super::callback::{CallbackTransform, CallbackVars};
use super::covered::SameKeyRotation;
use super::error::MigrationError;
use super::store::{CallbackTable, MigrationStore, SettingsRow};
use super::walkers::Walkers;

const VANTAGE_SETTINGS: &str = "vantage_settings";
const VANTAGE_FIELDS: &[&str] = &["api_key", "integration_token"];
const CLOUDZERO_SETTINGS: &str = "cloudzero_settings";
const CLOUDZERO_FIELDS: &[&str] = &["api_key"];

pub struct Migration<'run, S, R, T> {
    pub store: &'run S,
    pub rotation: &'run mut R,
    pub cipher: &'run DbCipher,
    pub transform: T,
    pub actor: Option<&'run str>,
    pub dry_run: bool,
}

impl<S, R, T> Migration<'_, S, R, T>
where
    S: MigrationStore,
    R: SameKeyRotation + Send + Sync,
    T: CallbackTransform + Send + Sync,
{
    pub async fn run(&mut self) -> Result<MigrationReport, MigrationError<S::Error, R::Error>> {
        if self.cipher.settings.algorithm != Algorithm::Aes256Gcm {
            return Err(MigrationError::AlgorithmGate);
        }
        let mut report = if self.dry_run {
            self.store
                .scan_covered(self.cipher)
                .await
                .map_err(MigrationError::Store)?
        } else {
            self.covered_tables().await?
        };
        report
            .locations
            .extend(self.walk().await.map_err(MigrationError::Store)?);
        Ok(report)
    }

    // A dry run only scans the rotation-covered stores; a real run scans, rotates with the same key, and scans again.
    async fn covered_tables(
        &mut self,
    ) -> Result<MigrationReport, MigrationError<S::Error, R::Error>> {
        let pre = self
            .store
            .scan_covered(self.cipher)
            .await
            .map_err(MigrationError::Store)?;
        let Some(key) = self.cipher.settings.signing_key.as_ref() else {
            return Err(MigrationError::MissingKey);
        };
        self.rotation
            .reencrypt_same_key(key, self.actor)
            .await
            .map_err(MigrationError::Rotation)?;
        let mut post = self
            .store
            .scan_covered(self.cipher)
            .await
            .map_err(MigrationError::Store)?;
        for location in &mut post.locations {
            let before = pre
                .locations
                .iter()
                .find(|report| report.location == location.location)
                .map_or(0_u64, |report| report.legacy);
            location.migrated = before.saturating_sub(location.legacy);
        }
        Ok(post)
    }

    async fn walk(&self) -> Result<Vec<LocationReport>, S::Error> {
        let walkers = Walkers {
            store: self.store,
            cipher: self.cipher,
        };
        let team = walkers
            .callback_vars(CallbackTable::Team, &self.transform, self.dry_run)
            .await?;
        let tokens = walkers
            .callback_vars(
                CallbackTable::VerificationToken,
                &self.transform,
                self.dry_run,
            )
            .await?;
        let vantage = walkers
            .settings(
                SettingsRow::Config {
                    param_name: VANTAGE_SETTINGS,
                    fields: VANTAGE_FIELDS,
                },
                self.dry_run,
            )
            .await?;
        let cloudzero = walkers
            .settings(
                SettingsRow::Config {
                    param_name: CLOUDZERO_SETTINGS,
                    fields: CLOUDZERO_FIELDS,
                },
                self.dry_run,
            )
            .await?;
        let sso = walkers.settings(SettingsRow::Sso, self.dry_run).await?;
        Ok(vec![team, tokens, vantage, cloudzero, sso])
    }
}

// check_encryption attests every location read-only and never requires the AES write gate.
pub async fn check_encryption<S>(store: &S, cipher: &DbCipher) -> Result<MigrationReport, S::Error>
where
    S: MigrationStore,
{
    let mut report = store.scan_covered(cipher).await?;
    let walkers = Walkers { store, cipher };
    let team = walkers
        .callback_vars(CallbackTable::Team, &CallbackVars, true)
        .await?;
    let tokens = walkers
        .callback_vars(CallbackTable::VerificationToken, &CallbackVars, true)
        .await?;
    let vantage = walkers
        .settings(
            SettingsRow::Config {
                param_name: VANTAGE_SETTINGS,
                fields: VANTAGE_FIELDS,
            },
            true,
        )
        .await?;
    let cloudzero = walkers
        .settings(
            SettingsRow::Config {
                param_name: CLOUDZERO_SETTINGS,
                fields: CLOUDZERO_FIELDS,
            },
            true,
        )
        .await?;
    let sso = walkers.settings(SettingsRow::Sso, true).await?;
    report
        .locations
        .extend([team, tokens, vantage, cloudzero, sso]);
    Ok(report)
}
