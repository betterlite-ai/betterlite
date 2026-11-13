#![no_main]

use betterlite_secrets::secret_codec::bearer::decrypt_bearer_token;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::secret_map::decode_secret_map;
use betterlite_secrets::secret_codec::{
    Algorithm, CipherSettings, DecryptPolicy, FailureLog, FailureValue, OsNonces, StoredCipher,
    StoredValue, decrypt_if_encrypted_with,
};
use libfuzzer_sys::fuzz_target;
use secrecy::SecretString;
use std::str;

fuzz_target!(|bytes: &[u8]| {
    let Ok(value) = str::from_utf8(bytes) else {
        return;
    };
    let key = SecretString::from("salt-one");
    let cipher = StoredCipher {
        settings: CipherSettings {
            signing_key: Some(key.clone()),
            algorithm: Algorithm::Aes256Gcm,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    };
    let policy = DecryptPolicy {
        log: FailureLog::Error,
        value: FailureValue::Original,
    };
    let _decrypted = cipher.decrypt_value(
        StoredValue::<()>::Text(SecretString::from(value)),
        "fuzz",
        policy,
    );
    let _maybe_decrypted = decrypt_if_encrypted_with(value, &key);
    let _bearer = decrypt_bearer_token(&cipher, value, "litellm_login_");
    let _secret_map = decode_secret_map(&cipher, Some(value), "fuzz");
    if let Ok(text) = serde_json::to_string(value) {
        let _secret_map_quoted = decode_secret_map(&cipher, Some(&text), "fuzz");
    }
});
