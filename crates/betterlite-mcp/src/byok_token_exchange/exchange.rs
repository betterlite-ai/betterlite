mod tests;

use secrecy::SecretString;

use super::pkce::verify_pkce;
use super::ports::{AuthCodeLookup, AuthCodeRecord, ByokCredentialSink};
use super::session_jwt::issue_session_token;
use super::{TokenError, TokenForm, TokenGrant};

// byok_token's steps 1 to 12, in LiteLLM's order; only an expired code and a finished run remove it.
pub async fn exchange_authorization_code<C, S>(
    form: &TokenForm,
    codes: &C,
    credentials: Option<&S>,
    master_key: Option<&SecretString>,
    now: f64,
) -> Result<TokenGrant, TokenError>
where
    C: AuthCodeLookup,
    S: ByokCredentialSink,
{
    codes.purge_expired(now);
    if form.grant_type != "authorization_code" {
        return Err(TokenError::UnsupportedGrantType);
    }
    let record = lookup_code(form, codes, now)?;
    check_binding(form, &record)?;
    let Some(signing_key) = master_key else {
        return Err(TokenError::ServerError);
    };
    codes.remove(&form.code);
    persist(credentials, &record).await?;
    let (user_id, server_id) = (record.user_id.as_str(), record.server_id.as_str());
    let access_token = issue_session_token(user_id, server_id, signing_key, now).map_err(|error| {
        tracing::error!(
            "byok_token: failed to sign the session token for user={user_id} server={server_id}: {error}"
        );
        TokenError::ServerError
    })?;
    Ok(TokenGrant {
        access_token,
        token_type: "bearer",
        expires_in: 3600,
    })
}

fn lookup_code<C>(form: &TokenForm, codes: &C, now: f64) -> Result<AuthCodeRecord, TokenError>
where
    C: AuthCodeLookup,
{
    let Some(record) = codes.get(&form.code) else {
        return Err(TokenError::InvalidGrant);
    };
    if now > record.expires_at {
        codes.remove(&form.code);
        return Err(TokenError::InvalidGrant);
    }
    Ok(record)
}

fn check_binding(form: &TokenForm, record: &AuthCodeRecord) -> Result<(), TokenError> {
    if !verify_pkce(&form.code_verifier, &record.code_challenge) {
        return Err(TokenError::InvalidGrant);
    }
    if !record.redirect_uri.is_empty()
        && !form.redirect_uri.is_empty()
        && form.redirect_uri != record.redirect_uri
    {
        return Err(TokenError::InvalidGrant);
    }
    if !record.client_id.is_empty() && form.client_id != record.client_id {
        return Err(TokenError::InvalidGrant);
    }
    if record.user_id.is_empty() {
        return Err(TokenError::InvalidGrant);
    }
    Ok(())
}

async fn persist<S>(credentials: Option<&S>, record: &AuthCodeRecord) -> Result<(), TokenError>
where
    S: ByokCredentialSink,
{
    let AuthCodeRecord {
        api_key,
        server_id,
        user_id,
        ..
    } = record;
    let Some(sink) = credentials else {
        tracing::warn!(
            "byok_token: the credential was not persisted for user={user_id} server={server_id} because no database is configured"
        );
        return Ok(());
    };
    if let Err(error) = sink.store_credential(user_id, server_id, api_key).await {
        tracing::error!(
            "byok_token: failed to store user credential for user={user_id} server={server_id}: {error}"
        );
        return Err(TokenError::ServerError);
    }
    if let Err(error) = sink.invalidate_cache(user_id, server_id).await {
        tracing::error!(
            "byok_token: failed to store user credential for user={user_id} server={server_id}: {error}"
        );
        return Err(TokenError::ServerError);
    }
    Ok(())
}
