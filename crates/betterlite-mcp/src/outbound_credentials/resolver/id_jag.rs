use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret, SecretString};
use sha2::{Digest as _, Sha256};
use tracing::warn;

use crate::credential_contracts::auth_config::id_jag::{ClientAuth, IdJagConfig};
use crate::credential_contracts::errors::{CredError, CredResult};
use crate::credential_contracts::server_spec::{ServerSpec, Subject};
use crate::credentials::ResolvedAuth;
use crate::sso_assertions::assertion::assertion_expired;

use crate::outbound_credentials::auth::static_auth;
use crate::outbound_credentials::collaborators::{
    AssertionStore, ExchangedToken, TokenEndpointClient,
};

use super::Collaborators;

const TOKEN_EXCHANGE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:token-exchange";
const JWT_BEARER_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:jwt-bearer";
const ID_JAG_REQUESTED_TOKEN_TYPE: &str = "urn:ietf:params:oauth:token-type:id-jag";

const MISSING_CALLER: &str = "ID-JAG requires an identified caller; this request carries neither an identity token nor a resolved LiteLLM user.";
const NO_ASSERTION: &str = "ID-JAG requires an IdP identity assertion for this user and none is stored. Sign in through LiteLLM SSO so the gateway captures one.";
const EXPIRED_ASSERTION: &str = "The stored IdP identity assertion for this user has expired. Sign in through LiteLLM SSO again to capture a current one.";

// Every miss is 412: ID-JAG exists to assert one user, so a missing subject has no safe substitute, while a store outage is the caller's 503.
pub(super) async fn subject_token(
    subject: &Subject,
    store: &dyn AssertionStore,
    now: DateTime<Utc>,
) -> CredResult<SecretString> {
    if let Some(inbound) = &subject.inbound_token {
        return Ok(inbound.clone());
    }
    if subject.subject_id.is_empty() {
        return Err(CredError::PreconditionRequired(MISSING_CALLER.to_owned()));
    }
    let fetched = store.fetch(&subject.subject_id).await;
    let assertion = match fetched {
        Ok(Some(assertion)) => assertion,
        Ok(None) => return Err(CredError::PreconditionRequired(NO_ASSERTION.to_owned())),
        Err(_unavailable) => {
            warn!(
                user_id = %subject.subject_id,
                "ID-JAG: the IdP identity assertion store is unreachable"
            );
            return Err(CredError::UpstreamUnavailable(
                "The IdP identity assertion store is unreachable, so ID-JAG cannot resolve a subject."
                    .to_owned(),
            ));
        }
    };
    if assertion_expired(&assertion, now) {
        return Err(CredError::PreconditionRequired(
            EXPIRED_ASSERTION.to_owned(),
        ));
    }
    Ok(assertion.id_token)
}

pub(super) async fn exchange(
    subject: &Subject,
    server: &ServerSpec,
    config: &IdJagConfig,
    subject_token: &SecretString,
    collaborators: &Collaborators,
) -> CredResult<Arc<dyn ResolvedAuth>> {
    let slot = slot_key(subject, server);
    let fingerprint = fingerprint(subject_token, &server.server_id, config);
    let endpoint: &dyn TokenEndpointClient = &*collaborators.token_endpoint;
    let compute = exchange_legs(config, subject_token, endpoint);
    let access_token = collaborators
        .exchanged_tokens
        .get_or_compute(&slot, &fingerprint, compute)
        .await?;
    static_auth(&config.header, access_token.expose_secret())
}

async fn exchange_legs(
    config: &IdJagConfig,
    subject_token: &SecretString,
    endpoint: &dyn TokenEndpointClient,
) -> CredResult<ExchangedToken> {
    let first_leg = first_leg_params(config, subject_token);
    let assertion = endpoint
        .fetch(
            &config.org_token_endpoint,
            &config.client_id,
            &first_leg,
            &config.client_auth,
        )
        .await?;
    let second_leg = second_leg_params(&assertion.access_token);
    endpoint
        .fetch(
            &config.resource_token_endpoint,
            &config.client_id,
            &second_leg,
            &config.client_auth,
        )
        .await
}

fn first_leg_params(
    config: &IdJagConfig,
    subject_token: &SecretString,
) -> BTreeMap<String, String> {
    let mut params = BTreeMap::new();
    params.insert(
        "grant_type".to_owned(),
        TOKEN_EXCHANGE_GRANT_TYPE.to_owned(),
    );
    params.insert(
        "requested_token_type".to_owned(),
        ID_JAG_REQUESTED_TOKEN_TYPE.to_owned(),
    );
    params.insert(
        "subject_token".to_owned(),
        subject_token.expose_secret().to_owned(),
    );
    params.insert(
        "subject_token_type".to_owned(),
        config.subject_token_type.clone(),
    );
    let audience = config.audience.as_deref().filter(|value| !value.is_empty());
    if let Some(configured) = audience {
        params.insert("audience".to_owned(), configured.to_owned());
    }
    let resource = config.resource.as_deref().filter(|value| !value.is_empty());
    if let Some(configured) = resource {
        params.insert("resource".to_owned(), configured.to_owned());
    }
    if !config.scopes.is_empty() {
        params.insert("scope".to_owned(), config.scopes.join(" "));
    }
    params
}

fn second_leg_params(assertion: &SecretString) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("grant_type".to_owned(), JWT_BEARER_GRANT_TYPE.to_owned()),
        ("assertion".to_owned(), assertion.expose_secret().to_owned()),
    ])
}

// The slot is addressed by the principal, never by the credential material, so invalidate stays computable while the assertion store is down.
pub(super) fn slot_key(subject: &Subject, server: &ServerSpec) -> String {
    let inbound = subject
        .inbound_token
        .as_ref()
        .map_or("", ExposeSecret::expose_secret);
    let material = format!(
        "{}\u{0}{}\u{0}{}\u{0}{inbound}",
        subject.tenant_id, subject.subject_id, server.server_id
    );
    hex::encode(Sha256::digest(material.as_bytes()))
}

// Everything an exchange depends on goes into the fingerprint, so a rotated assertion or an edited server re-mints instead of serving a bearer authorized under the old policy.
fn fingerprint(subject_token: &SecretString, server_id: &str, config: &IdJagConfig) -> String {
    let client_auth = client_auth_fingerprint(&config.client_auth);
    let material = format!(
        "{}\u{0}{server_id}\u{0}{}\u{0}{}\u{0}{}\u{0}{client_auth}\u{0}{}\u{0}{}\u{0}{}\u{0}{}",
        subject_token.expose_secret(),
        config.org_token_endpoint,
        config.resource_token_endpoint,
        config.client_id,
        config.subject_token_type,
        config.audience.as_deref().unwrap_or_default(),
        config.resource.as_deref().unwrap_or_default(),
        config.scopes.join(" "),
    );
    hex::encode(Sha256::digest(material.as_bytes()))
}

fn client_auth_fingerprint(client_auth: &ClientAuth) -> String {
    match client_auth {
        ClientAuth::PrivateKeyJwt(jwt) => format!(
            "private_key_jwt\u{0}{}\u{0}{}\u{0}{}",
            jwt.private_key.expose_secret(),
            jwt.key_id.as_deref().unwrap_or_default(),
            jwt.signing_alg
        ),
        ClientAuth::ClientSecret(secret) => {
            format!("client_secret\u{0}{}", secret.client_secret.expose_secret())
        }
    }
}
