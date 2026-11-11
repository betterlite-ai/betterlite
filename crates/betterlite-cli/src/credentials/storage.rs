use std::path::Path;

use async_trait::async_trait;
use secrecy::SecretString;

use crate::credential_lifecycle::files::CredentialFiles;
use crate::credential_lifecycle::load::load_cli_token;
use crate::credential_lifecycle::outcome::SecretSave;
use crate::credential_lifecycle::save::save_cli_token;
use crate::keyring::SecretVault;
use crate::pkce_login::{Http, WallClock};

use super::contracts::CliTokenData;
use super::notice::MessageSink;
use super::renewal::{FreshApiKeyRequest, RenewalVault, fresh_api_key};

pub struct CredentialStore<'store> {
    pub files: &'store dyn CredentialFiles,
    pub vault: &'store mut (dyn SecretVault + Send + Sync),
}

#[async_trait]
pub trait TokenStore: Send {
    async fn load_token(&mut self) -> Option<CliTokenData>;

    async fn save_token(&mut self, record: CliTokenData) -> SecretSave;
}

#[async_trait]
impl TokenStore for CredentialStore<'_> {
    async fn load_token(&mut self) -> Option<CliTokenData> {
        load_cli_token(self.files, self.vault).await
    }

    async fn save_token(&mut self, record: CliTokenData) -> SecretSave {
        save_cli_token(self.files, self.vault, record).await
    }
}

pub struct StoredApiKeyRequest<'store, H, W> {
    pub expected_base_url: Option<&'store str>,
    pub files: &'store dyn CredentialFiles,
    pub vault: &'store mut (dyn SecretVault + Send + Sync),
    pub http: &'store H,
    pub clock: &'store W,
    pub warn: MessageSink<'store>,
    pub token_path: &'store Path,
}

#[must_use]
pub async fn get_stored_api_key<H, W>(
    request: StoredApiKeyRequest<'_, H, W>,
) -> Option<SecretString>
where
    H: Http + Sync,
    W: WallClock + Sync,
{
    let mut store = CredentialStore {
        files: request.files,
        vault: request.vault,
    };
    let record = store.load_token().await?;
    if let Some(expected) = request.expected_base_url
        && record.base_url != expected.trim_end_matches('/')
    {
        return None;
    }
    let mut renewal = RenewalVault {
        store,
        warn: request.warn,
        token_path: request.token_path,
    };
    fresh_api_key(FreshApiKeyRequest {
        record: &record,
        store: &mut renewal,
        http: request.http,
        clock: request.clock,
        warn: request.warn,
    })
    .await
}
