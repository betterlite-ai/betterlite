#![no_main]

use betterlite_secrets::secret_codec::bearer::decrypt_bearer_token;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::secret_map::{StoredMap, decode_secret_map};
use betterlite_secrets::secret_codec::{
    Algorithm, CipherSettings, DecryptPolicy, FailureLog, FailureValue, OsNonces, StoredCipher,
    StoredValue, decrypt_if_encrypted_with,
};
use libfuzzer_sys::fuzz_target;
use secrecy::SecretString;

fuzz_target!(|bytes: &[u8]| {
    let Ok(value) = std::str::from_utf8(bytes) else {
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
    let stored: Option<StoredMap> = serde_json::from_str(value).ok();
    let _ = cipher.decrypt_value(StoredValue::<()>::Text(value.to_owned()), "fuzz", policy);
    let _ = decrypt_if_encrypted_with(value, &key);
    let _ = decrypt_bearer_token(&cipher, value, "litellm_login_");
    let _ = decode_secret_map(&cipher, stored.as_ref(), "fuzz");
});
