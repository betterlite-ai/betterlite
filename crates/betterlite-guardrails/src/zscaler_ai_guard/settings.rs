use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use secrecy::SecretString;

use super::error::ZscalerAiGuardError;
use super::init::ZscalerArgs;
use super::{
    API_KEY_ENV, DEFAULT_TIMEOUT, DEFAULT_URL, POLICY_ID_ENV, SEND_ALIAS_ENV, SEND_TEAM_ID_ENV,
    SEND_USER_ID_ENV, URL_ENV,
};

pub const DEFAULT_TIMEOUT_DURATION: Duration = Duration::from_secs(5);
pub const DEFAULT_POLICY_ID: i64 = -1;

// clippy allows two bools per struct, so the three send flags sit together
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SendFlags {
    pub alias: bool,
    pub user_id: bool,
    pub team_id: bool,
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub url: String,
    pub api_key: Option<SecretString>,
    pub policy_id: i64,
    pub send: SendFlags,
    pub timeout: Duration,
}

impl Settings {
    pub fn resolve<L>(args: &ZscalerArgs, environment: &L) -> Result<Self, ZscalerAiGuardError>
    where
        L: EnvironmentLookup,
    {
        let url = args
            .api_base
            .clone()
            .or_else(|| {
                environment
                    .lookup(URL_ENV)
                    .filter(|value| !value.is_empty())
            })
            .unwrap_or_else(|| DEFAULT_URL.to_owned());
        let api_key = args.api_key.clone().or_else(|| {
            environment
                .lookup(API_KEY_ENV)
                .filter(|value| !value.is_empty())
                .map(SecretString::from)
        });
        let policy_id = if let Some(id) = args.policy_id {
            id
        } else {
            let text = environment.lookup(POLICY_ID_ENV).unwrap_or_default();
            betterlite_env::number::<i64>(POLICY_ID_ENV, &text)
                .map_err(|source| ZscalerAiGuardError::PolicyId { source })?
                .unwrap_or(DEFAULT_POLICY_ID)
        };
        Ok(Self {
            url,
            api_key,
            policy_id,
            send: resolve_send(args, environment)?,
            timeout: resolve_timeout(args.timeout)?,
        })
    }
}

fn resolve_send<L>(args: &ZscalerArgs, environment: &L) -> Result<SendFlags, ZscalerAiGuardError>
where
    L: EnvironmentLookup,
{
    Ok(SendFlags {
        alias: resolve_flag(args.send_user_api_key_alias, environment, SEND_ALIAS_ENV)?,
        user_id: resolve_flag(
            args.send_user_api_key_user_id,
            environment,
            SEND_USER_ID_ENV,
        )?,
        team_id: resolve_flag(
            args.send_user_api_key_team_id,
            environment,
            SEND_TEAM_ID_ENV,
        )?,
    })
}

fn resolve_flag<L>(
    explicit: Option<bool>,
    environment: &L,
    name: &str,
) -> Result<bool, ZscalerAiGuardError>
where
    L: EnvironmentLookup,
{
    if let Some(value) = explicit {
        return Ok(value);
    }
    let Some(text) = environment.lookup(name) else {
        return Ok(false);
    };
    let parsed = betterlite_env::flag(name, &text)
        .map_err(|source| ZscalerAiGuardError::SendFlag { source })?;
    Ok(parsed.unwrap_or(false))
}

pub(crate) fn resolve_timeout(seconds: Option<f64>) -> Result<Duration, ZscalerAiGuardError> {
    match seconds {
        None => Ok(DEFAULT_TIMEOUT_DURATION),
        Some(value) if value > 0.0 => Duration::try_from_secs_f64(value)
            .map_err(|source| ZscalerAiGuardError::Timeout { source }),
        Some(value) => {
            tracing::warn!(
                "Ignoring non-positive Zscaler AI Guard timeout {value}, using {DEFAULT_TIMEOUT} seconds"
            );
            Ok(DEFAULT_TIMEOUT_DURATION)
        }
    }
}
