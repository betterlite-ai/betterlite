use crate::credential_lifecycle::files::CredentialFiles;
use crate::credential_lifecycle::load::load_cli_token;
use crate::credential_lifecycle::outcome::SecretSave;
use crate::credential_lifecycle::save::save_cli_token;
use crate::keyring::SecretVault;
use crate::pkce_login::Http;
use crate::pkce_login::revoke::{RevocationUnavailableOrFailure, revoke_credential};

use super::contracts::CliTokenData;
use super::notice::MessageSink;
use super::renewal::{refresh_inputs, renewal_contract};

pub struct ReplaceStoredTokenRequest<'store, H> {
    pub record: CliTokenData,
    pub files: &'store dyn CredentialFiles,
    pub vault: &'store mut (dyn SecretVault + Send + Sync),
    pub http: &'store H,
    pub echo: MessageSink<'store>,
}

#[must_use]
pub async fn replace_stored_token<H>(request: ReplaceStoredTokenRequest<'_, H>) -> SecretSave
where
    H: Http + Sync,
{
    let previous = load_cli_token(request.files, request.vault).await;
    let stored = save_cli_token(request.files, request.vault, request.record).await;
    if let Some(previous_login) = previous
        && !matches!(&stored, SecretSave::CredentialNotSaved(_))
        && let Some(failure) = revoke_stored_credential(&previous_login, request.http)
    {
        (request.echo)(&format!(
            "Could not revoke the previous login's refresh token on the proxy ({}); it expires on its own.",
            revocation_reason(&failure)
        ));
    }
    stored
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

fn revocation_reason(failure: &RevocationUnavailableOrFailure) -> String {
    match failure {
        RevocationUnavailableOrFailure::Unavailable(unavailable) => unavailable.reason.clone(),
        RevocationUnavailableOrFailure::Failure(failed) => failed.to_string(),
    }
}
