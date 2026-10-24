use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use secrecy::{ExposeSecret as _, SecretString};

use crate::error::SecretError;
use crate::secret_codec::python_base64::decode_strict;
use crate::secret_codec::sealing::{open, seal};
use crate::secret_codec::{Algorithm, NonceSource, StoredCipher};

// bearers are AES-GCM whatever the stored-value algorithm, with the prefix as AAD so a token can't change kind
pub fn encrypt_bearer_token<N, D>(
    cipher: &StoredCipher<N, D>,
    value: &SecretString,
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
    let sealed = seal(
        Algorithm::Aes256Gcm,
        key,
        &cipher.nonces,
        value.expose_secret().as_bytes(),
        prefix.as_bytes(),
    )?;
    Ok(format!("{prefix}{}", URL_SAFE_NO_PAD.encode(sealed)))
}

#[must_use]
pub fn decrypt_bearer_token<N, D>(
    cipher: &StoredCipher<N, D>,
    token: &str,
    prefix: &str,
) -> Option<SecretString> {
    let key = cipher.settings.signing_key.as_ref()?;
    let sealed = decode_strict(token.strip_prefix(prefix)?).ok()?;
    open(Algorithm::Aes256Gcm, key, &sealed, prefix.as_bytes()).ok()
}
