pub mod app_crypto;

use secrecy::SecretString;

use crate::secret_codec::diagnostics::SecretDiagnostics;
use crate::secret_codec::secret_map::{self, SecretMap};
use crate::secret_codec::{
    DecryptPolicy, FailureLog, FailureValue, NonceSource, StoredCipher, StoredValue,
};

/// Decode a raw byte slice as a stored scalar secret, the shape a fuzz
/// corpus or a database column hands back before any UTF-8 check.
pub fn decode_stored_secret<N, D>(cipher: &StoredCipher<N, D>, bytes: &[u8]) -> Option<SecretString>
where
    N: NonceSource,
    D: SecretDiagnostics,
{
    let text = str::from_utf8(bytes).ok()?;
    let policy = DecryptPolicy {
        log: FailureLog::Debug,
        value: FailureValue::Nothing,
    };
    match cipher.decrypt_value(
        StoredValue::<()>::Text(SecretString::from(text)),
        "fuzz",
        policy,
    ) {
        Some(StoredValue::Text(opened)) => Some(opened),
        Some(StoredValue::Other(())) | None => None,
    }
}

/// Decode a raw byte slice as a stored secret map, the counterpart fuzz entry
/// point for `decode_stored_secret`.
pub fn decode_secret_map<N, D>(cipher: &StoredCipher<N, D>, bytes: &[u8]) -> Option<SecretMap>
where
    N: NonceSource,
    D: SecretDiagnostics,
{
    let text = str::from_utf8(bytes).ok()?;
    secret_map::decode_secret_map(cipher, Some(text), "fuzz").ok()?
}

mod tests;
