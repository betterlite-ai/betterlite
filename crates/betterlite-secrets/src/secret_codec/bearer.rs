use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

use crate::error::SecretError;
use crate::secret_codec::python_base64::decode_strict;
use crate::secret_codec::sealing::{open_aes, seal_aes};
use crate::secret_codec::{NonceSource, StoredCipher};

// bearers are AES-GCM whatever the stored-value algorithm, with the prefix as AAD so a token can't change kind
pub fn encrypt_bearer_token<N, D>(
    cipher: &StoredCipher<N, D>,
    value: &str,
    prefix: &str,
) -> Result<String, SecretError>
where
    N: NonceSource,
{
    let key = cipher
        .settings
        .signing_key
        .as_ref()
        .ok_or(SecretError::BearerSecret)?;
    let sealed = seal_aes(key, &cipher.nonces, value.as_bytes(), prefix.as_bytes())?;
    Ok(format!("{prefix}{}", URL_SAFE_NO_PAD.encode(sealed)))
}

#[must_use]
pub fn decrypt_bearer_token<N, D>(
    cipher: &StoredCipher<N, D>,
    token: &str,
    prefix: &str,
) -> Option<String> {
    let key = cipher.settings.signing_key.as_ref()?;
    let sealed = decode_strict(token.strip_prefix(prefix)?).ok()?;
    open_aes(key, &sealed, prefix.as_bytes()).ok()
}
