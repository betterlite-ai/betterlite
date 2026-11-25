#![no_main]

use std::str;

use betterlite_db::stored_cipher::DbCipher;
use betterlite_mcp::gateway_dcr::sealed::open_gateway_dcr_client;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredCipher};
use libfuzzer_sys::fuzz_target;
use secrecy::SecretString;

// the seeds are sealed with this key by LiteLLM itself, so they reach the model checks
const CORPUS_SALT: &str = "b4674-corpus-salt";

fn cipher() -> DbCipher {
    StoredCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from(CORPUS_SALT)),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

fuzz_target!(|bytes: &[u8]| {
    let Ok(text) = str::from_utf8(bytes) else {
        return;
    };
    let _opened = open_gateway_dcr_client(text, &cipher());
});
