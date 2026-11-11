use std::path::Path;

use async_trait::async_trait;
use num_traits::ToPrimitive as _;
use secrecy::{ExposeSecret as _, SecretString};
use subtle::ConstantTimeEq as _;

use crate::credential_lifecycle::outcome::SecretSave;
use crate::pkce_login::models::{CliAuthContract, PkceCredential, PkceFailure};
use crate::pkce_login::oauth::{RefreshCredentialRequest, refresh_credential};
use crate::pkce_login::{Http, WallClock};
use crate::token_records::freshness::is_token_fresh;

use super::contracts::CliTokenData;
use super::login::pkce_token_record;
use super::notice::{MessageSink, storage_notice};
use super::storage::{CredentialStore, TokenStore as _};

pub struct RefreshInputs {
    pub token_endpoint: String,
    pub revocation_endpoint: String,
    pub resource: String,
    pub client_id: String,
    pub refresh_token: SecretString,
}

#[must_use]
pub fn refresh_inputs(record: &CliTokenData) -> Option<RefreshInputs> {
    Some(RefreshInputs {
        token_endpoint: record
            .token_endpoint
            .as_deref()
            .filter(|value| !value.is_empty())?
            .to_owned(),
        revocation_endpoint: record
            .revocation_endpoint
            .as_deref()
            .filter(|value| !value.is_empty())?
            .to_owned(),
        resource: record
            .resource
            .as_deref()
            .filter(|value| !value.is_empty())?
            .to_owned(),
        client_id: record
            .client_id
            .as_deref()
            .filter(|value| !value.is_empty())?
            .to_owned(),
        refresh_token: record
            .refresh_token
            .clone()
            .filter(|value| !value.expose_secret().is_empty())?,
    })
}

// The refresh and revocation grants send only the token endpoint, the revocation endpoint and the resource.
pub(super) fn renewal_contract(inputs: &RefreshInputs, base_url: &str) -> CliAuthContract {
    CliAuthContract {
        contract_version: 1,
        issuer: base_url.to_owned(),
        authorization_endpoint: String::new(),
        token_endpoint: inputs.token_endpoint.clone(),
        registration_endpoint: String::new(),
        revocation_endpoint: inputs.revocation_endpoint.clone(),
        resource: inputs.resource.clone(),
        supported_methods: Vec::new(),
    }
}

#[async_trait]
pub trait RenewalStore: Send {
    async fn save(&mut self, record: CliTokenData) -> SecretSave;

    async fn reload(&mut self) -> Option<CliTokenData>;
}

pub struct RenewalVault<'store> {
    pub store: CredentialStore<'store>,
    pub warn: MessageSink<'store>,
    pub token_path: &'store Path,
}

#[async_trait]
impl RenewalStore for RenewalVault<'_> {
    async fn save(&mut self, record: CliTokenData) -> SecretSave {
        let outcome = self.store.save_token(record).await;
        if matches!(
            &outcome,
            SecretSave::CredentialNotSaved(_) | SecretSave::CredentialNotRecorded(_)
        ) {
            (self.warn)(&storage_notice(&outcome, self.token_path));
        }
        outcome
    }

    async fn reload(&mut self) -> Option<CliTokenData> {
        self.store.load_token().await
    }
}

pub struct FreshApiKeyRequest<'store, H, W> {
    pub record: &'store CliTokenData,
    pub store: &'store mut (dyn RenewalStore + Send),
    pub http: &'store H,
    pub clock: &'store W,
    pub warn: MessageSink<'store>,
}

#[must_use]
pub async fn fresh_api_key<H, W>(request: FreshApiKeyRequest<'_, H, W>) -> Option<SecretString>
where
    H: Http + Sync,
    W: WallClock + Sync,
{
    let key = request.record.key.as_ref()?;
    if key.expose_secret().is_empty() {
        return None;
    }
    let Some(expires_at) = request.record.expires_at else {
        return Some(key.clone());
    };
    let now = request.clock.now_seconds().to_f64().unwrap_or_default();
    // is_token_fresh ignores expiration_hours for a record that carries expires_at
    if is_token_fresh(request.record, now, 0) {
        return Some(key.clone());
    }
    let still_valid = (now < expires_at).then(|| key.clone());
    let Some(inputs) = refresh_inputs(request.record) else {
        return still_valid;
    };
    let refreshed = refresh_stored_credential(request.record, &inputs, request.http, request.clock);
    match refreshed {
        Ok(credential) => Some(
            save_renewed_credential(request.store, request.record, credential, request.clock).await,
        ),
        Err(failure) => {
            let sibling = sibling_after_failed_refresh(
                request.store,
                request.record,
                request.clock,
                request.warn,
                &failure,
            )
            .await;
            sibling.or(still_valid)
        }
    }
}

fn refresh_stored_credential<H, W>(
    record: &CliTokenData,
    inputs: &RefreshInputs,
    http: &H,
    clock: &W,
) -> Result<PkceCredential, PkceFailure>
where
    H: Http,
    W: WallClock,
{
    refresh_credential(
        &renewal_contract(inputs, &record.base_url),
        &RefreshCredentialRequest {
            client_id: inputs.client_id.clone(),
            refresh_token: inputs.refresh_token.clone(),
        },
        http,
        clock,
    )
}

async fn save_renewed_credential<W>(
    store: &mut (dyn RenewalStore + Send),
    record: &CliTokenData,
    credential: PkceCredential,
    clock: &W,
) -> SecretString
where
    W: WallClock + Sync,
{
    let renewed = pkce_token_record(&record.base_url, &credential, clock);
    store.save(renewed).await;
    credential.access_token
}

async fn sibling_after_failed_refresh<W>(
    store: &mut (dyn RenewalStore + Send),
    record: &CliTokenData,
    clock: &W,
    warn: MessageSink<'_>,
    failure: &PkceFailure,
) -> Option<SecretString>
where
    W: WallClock + Sync,
{
    let reloaded = store.reload().await;
    let now = clock.now_seconds().to_f64().unwrap_or_default();
    let sibling = key_rotated_by_a_sibling(reloaded.as_ref(), record, now);
    if sibling.is_none() {
        (warn)(&format!("Could not renew the key: {failure}"));
    }
    sibling
}

#[must_use]
pub fn key_rotated_by_a_sibling(
    reloaded: Option<&CliTokenData>,
    previous: &CliTokenData,
    now_seconds: f64,
) -> Option<SecretString> {
    let candidate = reloaded?;
    if same_secret(
        candidate.refresh_token.as_ref(),
        previous.refresh_token.as_ref(),
    ) {
        return None;
    }
    if candidate.base_url != previous.base_url
        || candidate.token_endpoint != previous.token_endpoint
        || candidate.resource != previous.resource
        || candidate.user_id != previous.user_id
        || candidate.team_id != previous.team_id
    {
        return None;
    }
    if now_seconds >= candidate.expires_at? {
        return None;
    }
    let key = candidate.key.as_ref()?;
    if key.expose_secret().is_empty() {
        return None;
    }
    Some(key.clone())
}

fn same_secret(left: Option<&SecretString>, right: Option<&SecretString>) -> bool {
    match (left, right) {
        (Some(left_token), Some(right_token)) => bool::from(
            left_token
                .expose_secret()
                .as_bytes()
                .ct_eq(right_token.expose_secret().as_bytes()),
        ),
        (None, None) => true,
        (Some(_), None) | (None, Some(_)) => false,
    }
}
