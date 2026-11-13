use std::io::Write as _;
use std::sync::Arc;

use num_traits::ToPrimitive as _;
use secrecy::SecretString;

use crate::agent_dispatch::error::AgentDispatchError;
use crate::agent_launch::Terminal;
use crate::credentials::storage::{StoredApiKeyRequest, get_stored_api_key};
use crate::login::error::LoginError;
use crate::pkce_login::{Clock, Entropy, Http, WallClock};
use crate::settings_backup::probes::{
    KeyResolvedRequest, key_resolved_on_the_way_in, stored_login_is_pkce, usable_login,
};
use crate::sso_polling::transport::PollTransport;

use super::error::UpError;
use super::ports::{LoginAttempt, UpFlow, UpPorts};
use super::runtime::block_on;

const LOGIN: &str = "lite login";
const PKCE_LOGIN: &str = "lite login --pkce";

pub fn ensure_fresh_login<H, E, W>(ports: &mut UpPorts<'_, H, E, W>) -> Result<(), UpError>
where
    H: Http + PollTransport + Send + Sync + 'static,
    E: Entropy,
    W: Clock + WallClock + Send + Sync + 'static,
{
    let base_url = ports.context.base_url.trim_end_matches('/').to_owned();
    let now = ports.clock.now_seconds().to_f64().unwrap_or_default();
    if usable_login_for_request(ports, &base_url, now)? {
        return Ok(());
    }
    let pkce = stored_login_is_pkce(ports.files, &mut *ports.vault)?;
    let command = if pkce { PKCE_LOGIN } else { LOGIN };
    if !ports.terminal.is_some_and(Terminal::is_interactive) {
        return Err(UpError::NoFreshLogin { command });
    }
    writeln!(
        ports.stdout,
        "No fresh LiteLLM login found for this proxy; starting login..."
    )
    .map_err(|source| UpError::Write { source })?;
    start_login(ports, pkce)?;
    if !usable_login_for_request(ports, &base_url, now)? {
        return Err(UpError::LoginProducedNoToken);
    }
    Ok(())
}

pub fn resolve_api_key<H, E, W>(ports: &mut UpPorts<'_, H, E, W>) -> Result<SecretString, UpError>
where
    H: Http + PollTransport + Send + Sync + 'static,
    E: Entropy,
    W: Clock + WallClock + Send + Sync + 'static,
{
    if let Some(key) = &ports.context.api_key {
        return Ok(key.clone());
    }
    if let Some(key) = stored_key(ports)? {
        return Ok(key);
    }
    if !ports.terminal.is_some_and(Terminal::is_interactive) {
        return Err(UpError::Login(AgentDispatchError::NoKey));
    }
    writeln!(
        ports.stdout,
        "No LiteLLM credentials found; starting login..."
    )
    .map_err(|source| UpError::Write { source })?;
    start_login(ports, false)?;
    stored_key(ports)?.ok_or_else(|| UpError::Login(AgentDispatchError::LoginProducedNoKey))
}

fn usable_login_for_request<H, E, W>(
    ports: &mut UpPorts<'_, H, E, W>,
    base_url: &str,
    now: f64,
) -> Result<bool, UpError>
where
    H: Http + Send + Sync + 'static,
    W: WallClock + Send + Sync + 'static,
{
    let resolved = key_resolved_on_the_way_in(KeyResolvedRequest {
        context: ports.context,
        base_url,
        files: ports.files,
        vault: &mut *ports.vault,
        http: Arc::clone(&ports.http),
        clock: Arc::clone(&ports.clock),
        warn: ports.warn,
        token_path: ports.token_path,
    })?;
    usable_login(
        resolved.as_ref(),
        ports.files,
        &mut *ports.vault,
        now,
        ports.expiration_hours,
    )
    .map_err(UpError::Backup)
}

fn stored_key<H, E, W>(ports: &mut UpPorts<'_, H, E, W>) -> Result<Option<SecretString>, UpError>
where
    H: Http + Send + Sync + 'static,
    W: WallClock + Send + Sync + 'static,
{
    let base_url = ports.context.base_url.clone();
    let found = block_on(get_stored_api_key(StoredApiKeyRequest {
        expected_base_url: Some(&base_url),
        files: ports.files,
        vault: &mut *ports.vault,
        http: Arc::clone(&ports.http),
        clock: Arc::clone(&ports.clock),
        warn: ports.warn,
        token_path: ports.token_path,
    }))
    .map_err(|source| UpError::Runtime { source })?;
    Ok(found)
}

fn start_login<H, E, W>(ports: &mut UpPorts<'_, H, E, W>, pkce: bool) -> Result<(), UpError>
where
    H: Http + PollTransport + Send + Sync + 'static,
    E: Entropy,
    W: Clock + WallClock + Send + Sync + 'static,
{
    let flow = UpFlow {
        context: ports.context,
        environment: ports.environment,
        files: ports.files,
        vault: &mut *ports.vault,
        private_files: ports.private_files,
        http: Arc::clone(&ports.http),
        clock: &*ports.clock,
        wall_clock: &*ports.clock,
        entropy: &mut *ports.entropy,
        sleeper: ports.sleeper,
        selection: &mut *ports.selection,
        browser: ports.browser,
        interrupt: ports.interrupt,
        token_path: ports.token_path,
        binary_path: ports.binary_path,
    };
    let attempt = LoginAttempt {
        pkce,
        flow,
        stdout: &mut *ports.stdout,
    };
    ports.login.start(attempt).map_err(UpError::LoginFlow)
}
