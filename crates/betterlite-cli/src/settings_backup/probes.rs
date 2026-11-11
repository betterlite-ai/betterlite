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

use super::error::BackupError;
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

pub fn usable_login(
    api_key: Option<&SecretString>,
    files: &dyn CredentialFiles,
    vault: &mut (dyn SecretVault + Send + Sync),
    now_seconds: f64,
    expiration_hours: i64,
) -> Result<bool, BackupError> {
    if api_key.is_none() {
        return Ok(false);
    }
    let stored =
        block_on(load_cli_token(files, vault)).map_err(|source| BackupError::Runtime { source })?;
    Ok(stored.is_some_and(|record| is_token_fresh(&record, now_seconds, expiration_hours)))
}

pub fn key_resolved_on_the_way_in<H, W>(
    request: KeyResolvedRequest<'_, H, W>,
) -> Result<Option<SecretString>, BackupError>
where
    H: Http + Send + Sync + 'static,
    W: WallClock + Send + Sync + 'static,
{
    if request.context.api_key_from_token_file {
        return Ok(request.context.api_key.clone());
    }
    block_on(get_stored_api_key(StoredApiKeyRequest {
        expected_base_url: Some(request.base_url),
        files: request.files,
        vault: request.vault,
        http: request.http,
        clock: request.clock,
        warn: request.warn,
        token_path: request.token_path,
    }))
    .map_err(|source| BackupError::Runtime { source })
}

pub fn stored_login_is_pkce(
    files: &dyn CredentialFiles,
    vault: &mut (dyn SecretVault + Send + Sync),
) -> Result<bool, BackupError> {
    let Some(record) =
        block_on(load_cli_token(files, vault)).map_err(|source| BackupError::Runtime { source })?
    else {
        return Ok(false);
    };
    Ok(record.refresh_token.is_some())
}
