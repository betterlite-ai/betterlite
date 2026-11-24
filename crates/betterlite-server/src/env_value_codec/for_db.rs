use betterlite_openai::openai_base_config::Object as JsonMap;
use betterlite_secrets::secret_codec::diagnostics::SecretDiagnostics;
use betterlite_secrets::secret_codec::{NonceSource, StoredCipher};
use secrecy::SecretString;

use super::{EnvValueCodecError, decrypt_env, encrypt_env};

// The Admin UI posts stored ciphertext back, so the decrypt pass runs first and the write seals each value once.
pub fn encrypt_env_for_db<N, D>(
    values: &JsonMap,
    cipher: &StoredCipher<N, D>,
    new_key: Option<&SecretString>,
) -> Result<JsonMap, EnvValueCodecError>
where
    N: NonceSource,
    D: SecretDiagnostics,
{
    let opened = decrypt_env(values, cipher);
    let sealed = encrypt_env(&opened, cipher, new_key)?;
    Ok(sealed)
}
