use std::sync::{Arc, Mutex};

use secrecy::SecretString;
use tokio::runtime::Builder;

use crate::credentials::storage::{StoredApiKeyRequest, get_stored_api_key};
use crate::login::echo::echo_line;
use crate::pkce_login::{Clock, Entropy, Http, WallClock};
use crate::sso_polling::transport::PollTransport;

use super::AgentPorts;
use super::error::AgentDispatchError;
use super::login_through;

pub fn resolve_api_key<H, E, W>(
    ports: &mut AgentPorts<'_, H, E, W>,
) -> Result<SecretString, AgentDispatchError>
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
    let Some(terminal) = ports.terminal else {
        return Err(AgentDispatchError::NoKey);
    };
    if !terminal.is_interactive() {
        return Err(AgentDispatchError::NoKey);
    }
    writeln!(
        ports.stdout,
        "No LiteLLM credentials found; starting login..."
    )?;
    login_through(ports)?;
    stored_key(ports)?.ok_or(AgentDispatchError::LoginProducedNoKey)
}

fn stored_key<H, E, W>(
    ports: &mut AgentPorts<'_, H, E, W>,
) -> Result<Option<SecretString>, AgentDispatchError>
where
    H: Http + Send + Sync + 'static,
    W: WallClock + Send + Sync + 'static,
{
    let base_url = &ports.context.base_url;
    let sink = Mutex::new(&mut *ports.stdout);
    let warn = |message: &str| echo_line(&sink, message);
    let runtime = Builder::new_current_thread()
        .build()
        .map_err(AgentDispatchError::Runtime)?;
    let found = runtime.block_on(get_stored_api_key(StoredApiKeyRequest {
        expected_base_url: Some(base_url),
        files: ports.files,
        vault: &mut *ports.vault,
        http: Arc::clone(&ports.http),
        clock: Arc::clone(&ports.clock),
        warn: &warn,
        token_path: ports.token_path,
    }));
    Ok(found)
}
