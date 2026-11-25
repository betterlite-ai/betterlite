use std::error::Error;
use std::future::Future;

use betterlite_db::encryption_scan::report::MigrationReport;
use betterlite_db::encryption_scan::{DbCipher, MigrationJson, ScanError};

// The two tables whose metadata carries callback_vars; the source reads them one at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallbackTable {
    Team,
    VerificationToken,
}

impl CallbackTable {
    #[must_use]
    pub const fn location(self) -> &'static str {
        match self {
            Self::Team => "team.callback_vars",
            Self::VerificationToken => "verification_token.callback_vars",
        }
    }
}

// metadata stays undecoded as Option so a NULL column and a stored JSON string are told apart.
#[derive(Debug, Clone)]
pub struct CallbackRow {
    pub key: String,
    pub metadata: Option<MigrationJson>,
}

// The rows the migration walks; every read and write fails on its own, so a failure propagates after the earlier writes committed.
pub trait MigrationStore: Sync {
    type Error: Error + Send + From<ScanError> + 'static;

    fn scan_covered(
        &self,
        cipher: &DbCipher,
    ) -> impl Future<Output = Result<MigrationReport, Self::Error>> + Send;

    fn config_row(
        &self,
        param_name: &str,
    ) -> impl Future<Output = Result<Option<MigrationJson>, Self::Error>> + Send;

    fn save_config_row(
        &self,
        param_name: &str,
        value: &MigrationJson,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    fn sso_row(&self) -> impl Future<Output = Result<Option<MigrationJson>, Self::Error>> + Send;

    fn save_sso_row(
        &self,
        value: &MigrationJson,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    fn callback_rows(
        &self,
        table: CallbackTable,
    ) -> impl Future<Output = Result<Vec<CallbackRow>, Self::Error>> + Send;

    fn save_callback_metadata(
        &self,
        table: CallbackTable,
        key: &str,
        metadata: &MigrationJson,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
}
