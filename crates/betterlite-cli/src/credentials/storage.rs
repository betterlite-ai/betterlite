use std::path::Path;
use std::sync::Arc;

use secrecy::SecretString;

use crate::credential_lifecycle::files::CredentialFiles;
use crate::credential_lifecycle::load::load_cli_token;
use crate::keyring::SecretVault;
use crate::pkce_login::{Http, WallClock};

use super::notice::MessageSink;
use super::renewal::{FreshApiKeyRequest, RenewalVault, fresh_api_key};

pub struct StoredApiKeyRequest<'store, H, W> {
    pub expected_base_url: Option<&'store str>,
    pub files: &'store dyn CredentialFiles,
    pub vault: &'store mut (dyn SecretVault + Send + Sync),
    pub http: Arc<H>,
    pub clock: Arc<W>,
    pub warn: MessageSink<'store>,
    pub token_path: &'store Path,
}

#[must_use]
pub async fn get_stored_api_key<H, W>(
    request: StoredApiKeyRequest<'_, H, W>,
) -> Option<SecretString>
where
    H: Http + Send + Sync + 'static,
    W: WallClock + Send + Sync + 'static,
{
    let record = load_cli_token(request.files, request.vault).await?;
    if let Some(expected) = request.expected_base_url
        && record.base_url != expected.trim_end_matches('/')
    {
        return None;
    }
    let mut renewal = RenewalVault {
        files: request.files,
        vault: request.vault,
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
