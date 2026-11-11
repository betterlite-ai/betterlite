use std::path::Path;
use std::sync::Arc;

use secrecy::SecretString;

use crate::cli_context::CliContext;
use crate::credential_lifecycle::files::CredentialFiles;
use crate::credential_lifecycle::load::load_cli_token;
use crate::credentials::notice::MessageSink;
use crate::credentials::storage::{StoredApiKeyRequest, get_stored_api_key};
use crate::keyring::SecretVault;
use crate::pkce_login::{Http, WallClock};
use crate::token_records::freshness::is_token_fresh;

use super::runtime::block_on;

pub struct KeyResolvedRequest<'request, H, W> {
    pub context: &'request CliContext,
    pub base_url: &'request str,
    pub files: &'request dyn CredentialFiles,
    pub vault: &'request mut (dyn SecretVault + Send + Sync),
    pub http: Arc<H>,
    pub clock: Arc<W>,
    pub warn: MessageSink<'request>,
    pub token_path: &'request Path,
}

#[must_use]
pub fn usable_login(
    api_key: Option<&SecretString>,
    files: &dyn CredentialFiles,
    vault: &mut (dyn SecretVault + Send + Sync),
    now_seconds: f64,
    expiration_hours: i64,
) -> bool {
    if api_key.is_none() {
        return false;
    }
    match block_on(load_cli_token(files, vault)) {
        Ok(Some(record)) => is_token_fresh(&record, now_seconds, expiration_hours),
        Ok(None) | Err(_) => false,
    }
}

#[must_use]
pub fn key_resolved_on_the_way_in<H, W>(
    request: KeyResolvedRequest<'_, H, W>,
) -> Option<SecretString>
where
    H: Http + Send + Sync + 'static,
    W: WallClock + Send + Sync + 'static,
{
    if request.context.api_key_from_token_file {
        return request.context.api_key.clone();
    }
    let loaded = block_on(get_stored_api_key(StoredApiKeyRequest {
        expected_base_url: Some(request.base_url),
        files: request.files,
        vault: request.vault,
        http: request.http,
        clock: request.clock,
        warn: request.warn,
        token_path: request.token_path,
    }));
    loaded.unwrap_or_default()
}

#[must_use]
pub fn stored_login_is_pkce(
    files: &dyn CredentialFiles,
    vault: &mut (dyn SecretVault + Send + Sync),
) -> bool {
    match block_on(load_cli_token(files, vault)) {
        Ok(Some(record)) => record.refresh_token.is_some(),
        Ok(None) | Err(_) => false,
    }
}
