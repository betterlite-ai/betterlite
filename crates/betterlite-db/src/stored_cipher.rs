use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{OsNonces, StoredCipher};

// B700's StoredCipher port: the one cipher every table repository shares.
pub type DbCipher = StoredCipher<OsNonces, TracingDiagnostics>;

mod tests;
