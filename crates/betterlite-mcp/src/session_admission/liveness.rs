use betterlite_auth::auth_key_lookup::KeyAuthenticator;
use secrecy::{ExposeSecret as _, SecretString};

// LiteLLM's _session_key_is_live: only a DB-backed sk- key can be revoked; other keys pass.
pub async fn session_key_is_live(
    authenticator: Option<&dyn KeyAuthenticator>,
    session_key: Option<&SecretString>,
) -> bool {
    let Some(key) = session_key else {
        return true;
    };
    if !key.expose_secret().starts_with("sk-") {
        return true;
    }
    let Some(lookup) = authenticator else {
        return true;
    };
    lookup.authenticate(key.expose_secret()).await.is_ok()
}
