use betterlite_auth::auth_key_lookup::KeyAuthenticator;

// LiteLLM's _session_key_is_live: only a DB-backed sk- key can be revoked; other keys pass.
pub async fn session_key_is_live(
    authenticator: Option<&dyn KeyAuthenticator>,
    session_key: Option<&str>,
) -> bool {
    let Some(key) = session_key else {
        return true;
    };
    if !key.starts_with("sk-") {
        return true;
    }
    let Some(authenticator) = authenticator else {
        return true;
    };
    authenticator.authenticate(key).await.is_ok()
}
