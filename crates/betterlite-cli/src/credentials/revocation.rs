use std::sync::Arc;

use crate::credential_lifecycle::files::CredentialFiles;
use crate::credential_lifecycle::load::load_cli_token;
use crate::credential_lifecycle::outcome::SecretSave;
use crate::credential_lifecycle::save::save_cli_token;
use crate::keyring::SecretVault;
use thiserror::Error;

use crate::pkce_login::Http;
use crate::pkce_login::revoke::{RevocationUnavailableOrFailure, revoke_credential};

use super::contracts::CliTokenData;
use super::notice::MessageSink;
use super::renewal::{refresh_inputs, renewal_contract};
use super::worker::{WorkerError, run_blocking};

pub struct ReplaceStoredTokenRequest<'store, H> {
    pub record: CliTokenData,
    pub files: &'store dyn CredentialFiles,
    pub vault: &'store mut (dyn SecretVault + Send + Sync),
    pub http: Arc<H>,
    pub echo: MessageSink<'store>,
}

#[must_use]
pub async fn replace_stored_token<H>(request: ReplaceStoredTokenRequest<'_, H>) -> SecretSave
where
    H: Http + Send + Sync + 'static,
{
    let previous = load_cli_token(request.files, request.vault).await;
    let stored = save_cli_token(request.files, request.vault, request.record).await;
    let revocation = if let Some(previous_login) = previous
        && !matches!(&stored, SecretSave::CredentialNotSaved(_))
    {
        revoke_off_thread(previous_login, request.http).await
    } else {
        None
    };
    if let Some(failure) = revocation {
        (request.echo)(&format!(
            "Could not revoke the previous login's refresh token on the proxy ({failure}); it expires on its own."
        ));
    }
    stored
}

#[derive(Debug, Error)]
pub enum RevocationFailure {
    #[error("{}", proxy_reason(.0))]
    Proxy(RevocationUnavailableOrFailure),
    #[error("the revocation worker failed: {source}")]
    Worker {
        #[source]
        source: WorkerError,
    },
}

async fn revoke_off_thread<H>(record: CliTokenData, http: Arc<H>) -> Option<RevocationFailure>
where
    H: Http + Send + Sync + 'static,
{
    let joined = run_blocking(move || revoke_stored_credential(&record, &*http)).await;
    match joined {
        Ok(None) => None,
        Ok(Some(failure)) => Some(RevocationFailure::Proxy(failure)),
        Err(source) => Some(RevocationFailure::Worker { source }),
    }
}

#[must_use]
pub fn revoke_stored_credential<H>(
    record: &CliTokenData,
    http: &H,
) -> Option<RevocationUnavailableOrFailure>
where
    H: Http,
{
    let inputs = refresh_inputs(record)?;
    revoke_credential(
        &renewal_contract(&inputs, &record.base_url),
        &inputs.client_id,
        &inputs.refresh_token,
        http,
    )
    .err()
}

fn proxy_reason(failure: &RevocationUnavailableOrFailure) -> String {
    match failure {
        RevocationUnavailableOrFailure::Unavailable(unavailable) => unavailable.reason.clone(),
        RevocationUnavailableOrFailure::Failure(failed) => failed.to_string(),
    }
}
