use std::sync::Arc;

use num_traits::ToPrimitive as _;
use secrecy::SecretString;

use crate::agent_dispatch::error::AgentDispatchError;
use crate::agent_launch::Terminal;
use crate::cli_context::CliContext;
use crate::pkce_login::{Clock, Entropy, Http, WallClock};
use crate::settings_backup::probes::{
    KeyResolvedRequest, key_resolved_on_the_way_in, stored_login_is_pkce, usable_login,
};
use crate::sso_polling::transport::PollTransport;

use super::error::UpError;
use super::ports::{LoginAttempt, UpFlow, UpPorts};

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
    if ports.usable_login_for_request(&base_url, now)? {
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
    let fresh = ports.key_after_login(&base_url)?;
    let usable = usable_login(
        fresh.as_ref(),
        ports.files,
        &mut *ports.vault,
        now,
        ports.expiration_hours,
    )
    .map_err(UpError::Backup)?;
    if !usable {
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
    if !ports.terminal.is_some_and(Terminal::is_interactive) {
        return Err(UpError::Login(AgentDispatchError::NoKey));
    }
    writeln!(
        ports.stdout,
        "No LiteLLM credentials found; starting login..."
    )
    .map_err(|source| UpError::Write { source })?;
    start_login(ports, false)?;
    let base_url = ports.context.base_url.clone();
    ports
        .key_after_login(&base_url)?
        .ok_or_else(|| UpError::Login(AgentDispatchError::LoginProducedNoKey))
}

impl<H, E, W> UpPorts<'_, H, E, W>
where
    H: Http + Send + Sync + 'static,
    W: WallClock + Send + Sync + 'static,
{
    fn resolved_key(
        &mut self,
        context: &CliContext,
        base_url: &str,
    ) -> Result<Option<SecretString>, UpError> {
        key_resolved_on_the_way_in(KeyResolvedRequest {
            context,
            base_url,
            files: self.files,
            vault: &mut *self.vault,
            http: Arc::clone(&self.http),
            clock: Arc::clone(&self.clock),
            warn: self.warn,
            token_path: self.token_path,
        })
        .map_err(UpError::Backup)
    }

    // The login stored a fresh token, so read it here; the token-file shortcut still holds the key from before
    fn key_after_login(&mut self, base_url: &str) -> Result<Option<SecretString>, UpError> {
        let mut context = self.context.clone();
        context.api_key_from_token_file = false;
        self.resolved_key(&context, base_url)
    }

    fn usable_login_for_request(&mut self, base_url: &str, now: f64) -> Result<bool, UpError> {
        let resolved = self.resolved_key(self.context, base_url)?;
        usable_login(
            resolved.as_ref(),
            self.files,
            &mut *self.vault,
            now,
            self.expiration_hours,
        )
        .map_err(UpError::Backup)
    }
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
