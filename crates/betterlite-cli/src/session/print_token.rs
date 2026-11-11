use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use num_traits::ToPrimitive as _;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::runtime::{Builder, Runtime};

use crate::cli_context::CliContext;
use crate::credential_lifecycle::files::CredentialFiles;
use crate::credential_lifecycle::load::load_cli_token;
use crate::credentials::notice::{MessageSink, keychain_unreadable_notice};
use crate::credentials::renewal::{FreshApiKeyRequest, RenewalVault, fresh_api_key};
use crate::keyring::SecretVault;
use crate::pkce_login::{Http, WallClock};
use crate::token_records::freshness::is_token_fresh;
use crate::token_records::record::CliTokenRecord;

use super::error::SessionError;
use super::login_command;

pub struct PrintTokenRequest<'request, H, W> {
    pub context: &'request CliContext,
    pub files: &'request dyn CredentialFiles,
    pub vault: &'request mut (dyn SecretVault + Send + Sync),
    pub http: Arc<H>,
    pub clock: Arc<W>,
    pub expiration_hours: i64,
    pub token_path: &'request Path,
    pub stdout: &'request mut dyn Write,
    pub stderr: &'request mut dyn Write,
}

pub fn print_token<H, W>(request: &mut PrintTokenRequest<'_, H, W>) -> Result<(), SessionError>
where
    H: Http + Send + Sync + 'static,
    W: WallClock + Send + Sync + 'static,
{
    let runtime = Builder::new_current_thread()
        .build()
        .map_err(|source| SessionError::Runtime { source })?;
    let warnings = Mutex::new(Vec::new());
    let warn = |message: &str| {
        warnings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message.to_owned());
    };
    let outcome = select_key(&runtime, request, &warn);
    let sink = &mut *request.stderr;
    let collected = warnings
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner);
    for warning in collected {
        let _unused = writeln!(sink, "{warning}");
    }
    let key = outcome?;
    let _key_write = request.stdout.write_all(key.expose_secret().as_bytes());
    let _newline = writeln!(request.stdout);
    Ok(())
}

fn select_key<H, W>(
    runtime: &Runtime,
    request: &mut PrintTokenRequest<'_, H, W>,
    warn: MessageSink<'_>,
) -> Result<SecretString, SessionError>
where
    H: Http + Send + Sync + 'static,
    W: WallClock + Send + Sync + 'static,
{
    let loaded = runtime.block_on(load_cli_token(request.files, &mut *request.vault));
    let Some(record) = loaded else {
        return Err(SessionError::NotAuthenticated);
    };
    let issued = record.base_url == request.context.base_url.trim_end_matches('/');
    if request.context.base_url_explicit && !issued {
        return Err(SessionError::NotAuthenticatedForServer);
    }
    let renews = record.refresh_token.is_some();
    let now = request.clock.now_seconds().to_f64().unwrap_or_default();
    if !is_token_fresh(&record, now, request.expiration_hours) && !renews {
        return Err(SessionError::TokenExpired);
    }
    if record.key.is_none() {
        return Err(SessionError::KeychainUnreadable {
            notice: keychain_unreadable_notice(&*request.vault),
        });
    }
    let selected = if issued && request.context.api_key_from_token_file {
        request.context.api_key.clone()
    } else {
        renew(runtime, &record, request, warn)
    };
    match selected {
        Some(found) if !found.expose_secret().is_empty() => Ok(found),
        Some(_) | None => Err(SessionError::UnusableKey {
            login_command: login_command(renews),
        }),
    }
}

fn renew<H, W>(
    runtime: &Runtime,
    record: &CliTokenRecord,
    request: &mut PrintTokenRequest<'_, H, W>,
    warn: MessageSink<'_>,
) -> Option<SecretString>
where
    H: Http + Send + Sync + 'static,
    W: WallClock + Send + Sync + 'static,
{
    let mut renewal = RenewalVault {
        files: request.files,
        vault: &mut *request.vault,
        warn,
        token_path: request.token_path,
    };
    runtime.block_on(fresh_api_key(FreshApiKeyRequest {
        record,
        store: &mut renewal,
        http: Arc::clone(&request.http),
        clock: Arc::clone(&request.clock),
        warn,
    }))
}
