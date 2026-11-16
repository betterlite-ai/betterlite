use std::env::current_exe;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use betterlite_config::loader::EnvironmentLookup;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};

use crate::agent_launch::EnvVars;
use crate::cli_context::CliContext;
use crate::commands::Client;
use crate::keyring::SecretVault;
use crate::pkce_login::models::PkceFailure;
use crate::pkce_login::{Clock, Entropy, WallClock};
use crate::token_records::file::token_file_path;

use super::Failure;
use super::streams::Streams;

const DEFAULT_EXPIRATION_HOURS: i64 = 24;
const EXPIRATION_ENV: [&str; 2] = [
    "CLI_JWT_EXPIRATION_HOURS",
    "LITELLM_CLI_JWT_EXPIRATION_HOURS",
];

pub(crate) struct CliClock;

impl Clock for CliClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

impl WallClock for CliClock {
    fn now_seconds(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| {
                i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
            })
    }
}

pub(crate) struct SystemEntropy;

impl Entropy for SystemEntropy {
    fn fill(&mut self, bytes: &mut [u8]) -> Result<(), PkceFailure> {
        getrandom::fill(bytes).map_err(|source| {
            PkceFailure::Protocol(format!(
                "the operating system could not provide secure randomness: {source}"
            ))
        })
    }
}

pub(crate) struct SessionParts<'parts> {
    pub(crate) context: &'parts CliContext,
    pub(crate) environment: &'parts (dyn EnvironmentLookup + Send + Sync),
    pub(crate) vault: &'parts mut (dyn SecretVault + Send + Sync),
    pub(crate) streams: Streams,
    pub(crate) client: Arc<Client>,
    pub(crate) clock: Arc<CliClock>,
    pub(crate) base_env: EnvVars,
    pub(crate) stdin_is_terminal: bool,
}

pub(crate) struct Session<'session> {
    pub(crate) context: CliContext,
    pub(crate) environment: &'session (dyn EnvironmentLookup + Send + Sync),
    pub(crate) vault: &'session mut (dyn SecretVault + Send + Sync),
    pub(crate) streams: Streams,
    pub(crate) client: Arc<Client>,
    pub(crate) clock: Arc<CliClock>,
    pub(crate) redactor: Redactor,
    pub(crate) home: Option<PathBuf>,
    pub(crate) token_path: Option<PathBuf>,
    pub(crate) binary_path: Option<PathBuf>,
    pub(crate) expiration_hours: i64,
    pub(crate) base_env: EnvVars,
    pub(crate) stdin_is_terminal: bool,
}

impl<'session> Session<'session> {
    pub(crate) fn new(parts: SessionParts<'session>) -> Result<Self, Failure> {
        let expiration_hours = expiration_hours(parts.environment)?;
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(|source| {
            Failure::Plain(format!("cannot build the secret redactor: {source}"))
        })?;
        let home = parts.environment.system_home();
        Ok(Self {
            context: parts.context.clone(),
            environment: parts.environment,
            vault: parts.vault,
            streams: parts.streams,
            client: parts.client,
            clock: parts.clock,
            redactor,
            token_path: home.as_deref().map(token_file_path),
            binary_path: current_exe().ok(),
            home,
            expiration_hours,
            base_env: parts.base_env,
            stdin_is_terminal: parts.stdin_is_terminal,
        })
    }
}

fn expiration_hours(environment: &dyn EnvironmentLookup) -> Result<i64, Failure> {
    for name in EXPIRATION_ENV {
        let Some(value) = environment.lookup(name).filter(|value| !value.is_empty()) else {
            continue;
        };
        return value.parse::<i64>().map_err(|_invalid| {
            Failure::Usage(format!("{name} must be an integer number of hours"))
        });
    }
    Ok(DEFAULT_EXPIRATION_HOURS)
}
