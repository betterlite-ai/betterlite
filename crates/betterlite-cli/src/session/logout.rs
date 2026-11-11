use std::io;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use tokio::runtime::Builder;

use crate::credential_lifecycle::clear::clear_cli_token;
use crate::credential_lifecycle::files::CredentialFiles;
use crate::credential_lifecycle::load::load_cli_token;
use crate::credential_lifecycle::outcome::SecretClear;
use crate::credentials::notice::{
    KEYRING_INSTALL_HINT, STRANDED_CREDENTIAL_MESSAGE, UNCHECKED_KEYCHAIN_MESSAGE,
};
use crate::credentials::revocation::revoke_stored_credential;
use crate::keyring::{DISABLE_KEYRING_ENV_VAR, SecretVault};
use crate::pkce_login::Http;
use crate::pkce_login::revoke::RevocationUnavailableOrFailure;
use crate::token_records::record::CliTokenRecord;

use super::error::SessionError;

pub const LOGOUT_ABOUT: &str = "Logout and clear stored authentication";

pub struct LogoutRequest<'request, H> {
    pub files: &'request dyn CredentialFiles,
    pub vault: &'request mut (dyn SecretVault + Send + Sync),
    pub http: Arc<H>,
    pub token_path: &'request Path,
    pub stdout: &'request mut dyn Write,
}

pub fn logout<H>(request: &mut LogoutRequest<'_, H>) -> Result<(), SessionError>
where
    H: Http + Send + Sync + 'static,
{
    let runtime = Builder::new_current_thread()
        .build()
        .map_err(|source| SessionError::Runtime { source })?;
    let stored = runtime.block_on(load_cli_token(request.files, &mut *request.vault));
    if let Some(record) = &stored {
        revoke_before_clearing(record, &*request.http, &mut *request.stdout)?;
    }
    let outcome = runtime.block_on(clear_cli_token(request.files, &mut *request.vault));
    let _rendered = render_clear_outcome(&outcome, request.token_path, &mut *request.stdout);
    Ok(())
}

fn revoke_before_clearing<H>(
    record: &CliTokenRecord,
    http: &H,
    stdout: &mut dyn Write,
) -> Result<(), SessionError>
where
    H: Http,
{
    match revoke_stored_credential(record, http) {
        Some(RevocationUnavailableOrFailure::Unavailable(unavailable)) => {
            Err(SessionError::RevocationUnavailable {
                reason: unavailable.reason,
            })
        }
        Some(RevocationUnavailableOrFailure::Failure(failure)) => {
            let _unused = writeln!(
                stdout,
                "Could not revoke the refresh token on the proxy ({failure}); it expires on its own."
            );
            Ok(())
        }
        None => Ok(()),
    }
}

const UNLOCK_AND_RETRY: &str = "Unlock your keychain and run 'lite logout' again to clear it.";

fn render_clear_outcome(
    outcome: &SecretClear,
    token_path: &Path,
    stdout: &mut dyn Write,
) -> io::Result<()> {
    let path = token_path.display();
    match outcome {
        SecretClear::SecretErased => writeln!(
            stdout,
            "Logged out successfully. Authentication token cleared."
        ),
        SecretClear::CredentialNotCleared(not_cleared) => {
            writeln!(
                stdout,
                "Your credential is still in {path}, which could not be removed: {}.",
                not_cleared.source
            )?;
            writeln!(
                stdout,
                "Delete that file, or make the directory writable and run 'lite logout' again."
            )
        }
        SecretClear::SecretStranded => {
            writeln!(stdout, "{STRANDED_CREDENTIAL_MESSAGE}")?;
            writeln!(stdout, "{UNLOCK_AND_RETRY}")
        }
        SecretClear::KeyringNotInstalled => unchecked_lines(
            stdout,
            &format!(
                "Install the keyring package with: {KEYRING_INSTALL_HINT}, then run 'lite logout' again."
            ),
        ),
        SecretClear::KeyringDisabled => unchecked_lines(
            stdout,
            &format!("Unset {DISABLE_KEYRING_ENV_VAR} and run 'lite logout' again to clear it."),
        ),
        SecretClear::KeyringUnreachable => unchecked_lines(stdout, UNLOCK_AND_RETRY),
    }
}

fn unchecked_lines(stdout: &mut dyn Write, second: &str) -> io::Result<()> {
    writeln!(stdout, "{UNCHECKED_KEYCHAIN_MESSAGE}")?;
    writeln!(stdout, "{second}")
}
