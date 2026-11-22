#![no_main]

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::responses_id_security::owned_id::decode_owned_response_id;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredCipher};
use libfuzzer_sys::fuzz_target;
use secrecy::SecretString;

fuzz_target!(|bytes: &[u8]| {
    let cipher = StoredCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from("fuzz-salt")),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    };
    drop(decode_owned_response_id(
        bytes,
        &cipher,
        ApiLimits::default(),
    ));
    let tiny = ApiLimits {
        max_json_bytes: 8,
        max_event_bytes: 8,
        max_items: 1,
    };
    drop(decode_owned_response_id(bytes, &cipher, tiny));
});
