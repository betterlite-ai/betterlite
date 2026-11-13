#![no_main]

use betterlite_secrets::error::SecretError;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::secret_map;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredCipher};
use libfuzzer_sys::fuzz_target;
use secrecy::SecretString;

const MAX_BYTES: usize = 4096;

fuzz_target!(|data: &[u8]| {
    let Some((&tag, rest)) = data.split_first() else {
        return;
    };
    let Some(bytes) = rest.get(..rest.len().min(MAX_BYTES)) else {
        return;
    };
    let cipher = StoredCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from("salt-one")),
            algorithm: Algorithm::Aes256Gcm,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    };
    if tag.is_multiple_of(2) {
        let _outcome = cipher.decode_stored_secret(bytes);
    } else {
        let _outcome = str::from_utf8(bytes)
            .map_err(|source| SecretError::Utf8 { source })
            .and_then(|text| secret_map::decode_secret_map(&cipher, Some(text), "fuzz"));
    }
});
