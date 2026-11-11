use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use num_traits::ToPrimitive as _;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

use crate::credential_lifecycle::files::CredentialFiles;
use crate::credential_lifecycle::load::load_cli_token;
use crate::credential_lifecycle::outcome::SecretSave;
use crate::credential_lifecycle::save::save_cli_token;
use crate::keyring::SecretVault;
use crate::pkce_login::models::{CliAuthContract, PkceCredential, PkceFailure};
use crate::pkce_login::oauth::{RefreshCredentialRequest, refresh_credential};
use crate::pkce_login::{Http, WallClock};
use crate::token_records::freshness::is_token_fresh;

use super::contracts::CliTokenData;
use super::login::pkce_token_record;
use super::notice::{MessageSink, storage_notice};
use super::sibling::key_rotated_by_a_sibling;
use super::worker::{WorkerError, run_blocking};

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
    pub files: &'store dyn CredentialFiles,
    pub vault: &'store mut (dyn SecretVault + Send + Sync),
    pub warn: MessageSink<'store>,
    pub token_path: &'store Path,
}

#[async_trait]
impl RenewalStore for RenewalVault<'_> {
    async fn save(&mut self, record: CliTokenData) -> SecretSave {
        let outcome = save_cli_token(self.files, self.vault, record).await;
        if matches!(
            &outcome,
            SecretSave::CredentialNotSaved(_) | SecretSave::CredentialNotRecorded(_)
        ) {
            (self.warn)(&storage_notice(&outcome, self.token_path));
        }
        outcome
    }

    async fn reload(&mut self) -> Option<CliTokenData> {
        load_cli_token(self.files, self.vault).await
    }
}

enum StoredKey {
    Missing,
    Usable(SecretString),
    Renew { key: SecretString, expires_at: f64 },
}

fn classify_stored_key(record: &CliTokenData, now: f64) -> StoredKey {
    let Some(key) = record.key.as_ref() else {
        return StoredKey::Missing;
    };
    if key.expose_secret().is_empty() {
        return StoredKey::Missing;
    }
    let Some(expires_at) = record.expires_at else {
        return StoredKey::Usable(key.clone());
    };
    // is_token_fresh ignores expiration_hours for a record that carries expires_at
    if is_token_fresh(record, now, 0) {
        return StoredKey::Usable(key.clone());
    }
    StoredKey::Renew {
        key: key.clone(),
        expires_at,
    }
}

pub struct FreshApiKeyRequest<'store, H, W> {
    pub record: &'store CliTokenData,
    pub store: &'store mut (dyn RenewalStore + Send),
    pub http: Arc<H>,
    pub clock: Arc<W>,
    pub warn: MessageSink<'store>,
}

#[must_use]
pub async fn fresh_api_key<H, W>(request: FreshApiKeyRequest<'_, H, W>) -> Option<SecretString>
where
    H: Http + Send + Sync + 'static,
    W: WallClock + Send + Sync + 'static,
{
    let now = request.clock.now_seconds().to_f64().unwrap_or_default();
    let (key, expires_at) = match classify_stored_key(request.record, now) {
        StoredKey::Missing => return None,
        StoredKey::Usable(key) => return Some(key),
        StoredKey::Renew { key, expires_at } => (key, expires_at),
    };
    let still_valid = (now < expires_at).then_some(key);
    let Some(inputs) = refresh_inputs(request.record) else {
        return still_valid;
    };
    let base_url = request.record.base_url.clone();
    let refreshed =
        refresh_off_thread(base_url, inputs, request.http, Arc::clone(&request.clock)).await;
    match refreshed {
        Ok(credential) => Some(
            save_renewed_credential(request.store, request.record, credential, &*request.clock)
                .await,
        ),
        Err(failure) => {
            let sibling = sibling_after_failed_refresh(
                request.store,
                request.record,
                &*request.clock,
                request.warn,
                &failure,
            )
            .await;
            sibling.or(still_valid)
        }
    }
}

#[derive(Debug, Error)]
pub enum RenewalFailure {
    #[error(transparent)]
    Refresh(#[from] PkceFailure),
    #[error("the renewal worker failed: {source}")]
    Worker {
        #[source]
        source: WorkerError,
    },
}

async fn refresh_off_thread<H, W>(
    base_url: String,
    inputs: RefreshInputs,
    http: Arc<H>,
    clock: Arc<W>,
) -> Result<PkceCredential, RenewalFailure>
where
    H: Http + Send + Sync + 'static,
    W: WallClock + Send + Sync + 'static,
{
    let joined =
        run_blocking(move || refresh_stored_credential(&base_url, &inputs, &*http, &*clock)).await;
    match joined {
        Ok(result) => result.map_err(RenewalFailure::Refresh),
        Err(source) => Err(RenewalFailure::Worker { source }),
    }
}

fn refresh_stored_credential<H, W>(
    base_url: &str,
    inputs: &RefreshInputs,
    http: &H,
    clock: &W,
) -> Result<PkceCredential, PkceFailure>
where
    H: Http,
    W: WallClock,
{
    refresh_credential(
        &renewal_contract(inputs, base_url),
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
    failure: &RenewalFailure,
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
