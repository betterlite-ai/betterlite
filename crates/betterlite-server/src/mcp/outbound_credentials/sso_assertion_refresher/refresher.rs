mod tests;

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_auth::jwt_claims::TypedValue;
use betterlite_config::sections::settings::JsonValue;
use betterlite_mcp::credential_contracts::token_endpoint_auth::{
    TokenEndpointClientAuth, build_token_client_auth,
};
use betterlite_mcp::sso_assertions::assertion::{SsoIdentityAssertion, assertion_from_sso_login};
use betterlite_mcp::sso_assertions::read::AssertionStoreUnavailable;
use secrecy::{ExposeSecret as _, SecretString};
use subtle::ConstantTimeEq as _;

use super::client_config::{SsoClientConfig, SsoClientSource};
use super::failure::{RefreshFailure, RefreshFailureKind};
use super::transport::{RefreshResponseBody, RefreshTransport};

#[async_trait]
pub trait AssertionAccess: Send + Sync {
    async fn read(
        &self,
        user_id: &str,
    ) -> Result<Option<SsoIdentityAssertion>, AssertionStoreUnavailable>;

    async fn write(
        &self,
        user_id: &str,
        assertion: &SsoIdentityAssertion,
    ) -> Result<(), AssertionStoreUnavailable>;
}

pub struct AssertionRefresher<T, C, A> {
    pub transport: T,
    pub client_source: C,
    pub access: Arc<A>,
}

impl<T, C, A> AssertionRefresher<T, C, A>
where
    T: RefreshTransport,
    C: SsoClientSource,
    A: AssertionAccess,
{
    pub async fn refresh(
        &self,
        user_id: &str,
        assertion: &SsoIdentityAssertion,
    ) -> Result<SsoIdentityAssertion, RefreshFailure> {
        let (config, carried, client_auth) = self.client_setup(user_id, assertion)?;
        let mut form = BTreeMap::new();
        form.insert("grant_type".to_owned(), "refresh_token".to_owned());
        form.insert(
            "refresh_token".to_owned(),
            carried.expose_secret().to_owned(),
        );
        for (name, value) in &client_auth.body {
            form.insert(name.clone(), value.expose_secret().to_owned());
        }
        let headers = client_auth
            .headers
            .iter()
            .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
            .collect();
        let body = self
            .transport
            .post(&config.token_endpoint, &form, &headers)
            .await?;
        self.renewed_from(user_id, assertion, &body, carried.expose_secret())
            .await
    }

    // The no-refresh-token check comes first, so an assertion with neither a token nor a client names the offline_access fix.
    fn client_setup(
        &self,
        user_id: &str,
        assertion: &SsoIdentityAssertion,
    ) -> Result<(SsoClientConfig, SecretString, TokenEndpointClientAuth), RefreshFailure> {
        let Some(carried) = assertion.refresh_token.clone() else {
            tracing::warn!(
                "ID-JAG: the stored IdP identity assertion for user_id={user_id} has expired and no refresh token was captured with it, so it cannot be renewed without another interactive sign-in. Add 'offline_access' to GENERIC_SCOPE so the SSO login captures one."
            );
            return Err(RefreshFailure {
                kind: RefreshFailureKind::Rejected,
                detail: "no refresh token was captured at sign-in".to_owned(),
            });
        };
        let Some(config) = self.client_source.client() else {
            tracing::warn!(
                "ID-JAG: the stored IdP identity assertion for user_id={user_id} has expired and cannot be renewed because the generic SSO client is not configured (GENERIC_TOKEN_ENDPOINT, GENERIC_CLIENT_ID, GENERIC_CLIENT_SECRET)."
            );
            return Err(RefreshFailure {
                kind: RefreshFailureKind::Rejected,
                detail: "the generic SSO client is not configured".to_owned(),
            });
        };
        // The config already holds a non-empty id and secret, so this cannot fail; a failure is still the same settled refusal.
        let Ok(client_auth) = build_token_client_auth(
            Some(config.auth_method),
            Some(config.client_id.as_str()),
            Some(&config.client_secret),
        ) else {
            return Err(RefreshFailure {
                kind: RefreshFailureKind::Rejected,
                detail: "the generic SSO client is not configured".to_owned(),
            });
        };
        Ok((config, carried, client_auth))
    }

    async fn renewed_from(
        &self,
        user_id: &str,
        previous: &SsoIdentityAssertion,
        body: &RefreshResponseBody,
        carried: &str,
    ) -> Result<SsoIdentityAssertion, RefreshFailure> {
        let rotated = body
            .get("refresh_token")
            .and_then(JsonValue::as_str)
            .filter(|text| !text.is_empty());
        let id_token = body
            .get("id_token")
            .map(|json| TypedValue { json: json.clone() });
        let refresh_value = TypedValue {
            json: JsonValue::String(rotated.unwrap_or(carried).to_owned()),
        };
        let Some(renewed) = assertion_from_sso_login(id_token.as_ref(), Some(&refresh_value))
        else {
            tracing::warn!(
                "ID-JAG: the IdP accepted the refresh for user_id={user_id} but returned no usable id_token, so there is nothing to assert upstream. The SSO client's grant needs the 'openid' scope for the token endpoint to return one on a refresh."
            );
            return Err(RefreshFailure {
                kind: RefreshFailureKind::Rejected,
                detail: "the IdP's refresh response carried no usable id_token".to_owned(),
            });
        };
        self.store_renewal(user_id, previous, &renewed).await?;
        Ok(renewed)
    }

    async fn store_renewal(
        &self,
        user_id: &str,
        previous: &SsoIdentityAssertion,
        renewed: &SsoIdentityAssertion,
    ) -> Result<(), RefreshFailure> {
        let failure = match self.access.read(user_id).await {
            Ok(Some(current))
                if !bool::from(
                    current
                        .id_token
                        .expose_secret()
                        .as_bytes()
                        .ct_eq(previous.id_token.expose_secret().as_bytes()),
                ) =>
            {
                tracing::info!(
                    "ID-JAG: a newer IdP identity assertion for user_id={user_id} was stored while this renewal was in flight; keeping the stored one."
                );
                return Ok(());
            }
            Ok(_unchanged) => match self.access.write(user_id, renewed).await {
                Ok(()) => return Ok(()),
                Err(failure) => failure,
            },
            Err(failure) => failure,
        };
        tracing::warn!(
            "ID-JAG: could not persist the renewed IdP identity assertion for user_id={user_id}, so the rotated refresh token is lost and this user will have to sign in again once the renewed token expires: {failure}"
        );
        Err(RefreshFailure {
            kind: RefreshFailureKind::Unavailable,
            detail: "the renewed IdP identity assertion could not be persisted".to_owned(),
        })
    }
}
