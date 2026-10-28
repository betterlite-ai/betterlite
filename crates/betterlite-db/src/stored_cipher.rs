use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredCipher};
use secrecy::SecretString;

/// Port of B700's `StoredCipher`.
///
/// The one cipher every table repository shares, built once from
/// `betterlite-config`'s resolved signing key and algorithm. This is an
/// adapter around `secret_codec::StoredCipher`, not a second implementation
/// of the wire format it already owns.
pub type DbCipher = StoredCipher<OsNonces, TracingDiagnostics>;

#[must_use]
pub const fn new_db_cipher(signing_key: Option<SecretString>, algorithm: Algorithm) -> DbCipher {
    StoredCipher {
        settings: CipherSettings {
            signing_key,
            algorithm,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

mod tests;
