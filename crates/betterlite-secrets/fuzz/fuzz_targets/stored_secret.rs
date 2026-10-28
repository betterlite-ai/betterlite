#![no_main]

use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredCipher};
use betterlite_secrets::stored_cipher::{decode_secret_map, decode_stored_secret};
use libfuzzer_sys::fuzz_target;
use secrecy::SecretString;

const MAX_BYTES: usize = 4096;

fuzz_target!(|data: &[u8]| {
    let Some((&tag, rest)) = data.split_first() else {
        return;
    };
    let bytes = &rest[..rest.len().min(MAX_BYTES)];
    let cipher = StoredCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from("salt-one")),
            algorithm: Algorithm::Aes256Gcm,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    };
    if tag.is_multiple_of(2) {
        let _ = decode_stored_secret(&cipher, bytes);
    } else {
        let _ = decode_secret_map(&cipher, bytes);
    }
});
