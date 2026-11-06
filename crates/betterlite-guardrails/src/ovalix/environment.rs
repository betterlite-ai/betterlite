use betterlite_config::loader::EnvironmentLookup;
use secrecy::SecretString;
use thiserror::Error;

use super::{CallHook, Params};

pub const TRACKER_API_BASE_ENV: &str = "OVALIX_TRACKER_API_BASE";
pub const TRACKER_API_KEY_ENV: &str = "OVALIX_TRACKER_API_KEY";
pub const APPLICATION_ID_ENV: &str = "OVALIX_APPLICATION_ID";
pub const PRE_CHECKPOINT_ID_ENV: &str = "OVALIX_PRE_CHECKPOINT_ID";
pub const POST_CHECKPOINT_ID_ENV: &str = "OVALIX_POST_CHECKPOINT_ID";

const TRACKER_API_BASE_SETTING: &str = "tracker_api_base (OVALIX_TRACKER_API_BASE)";
const TRACKER_API_KEY_SETTING: &str = "tracker_api_key (OVALIX_TRACKER_API_KEY)";
const APPLICATION_ID_SETTING: &str = "application_id (OVALIX_APPLICATION_ID)";
const PRE_CHECKPOINT_SETTING: &str = "pre_checkpoint_id (OVALIX_PRE_CHECKPOINT_ID)";
const POST_CHECKPOINT_SETTING: &str = "post_checkpoint_id (OVALIX_POST_CHECKPOINT_ID)";
const ANY_CHECKPOINT_SETTING: &str = "pre_checkpoint_id or post_checkpoint_id";

#[derive(Debug, Clone)]
pub struct Settings {
    pub tracker_api_base: String,
    pub tracker_api_key: SecretString,
    pub application_id: String,
    pub pre_checkpoint_id: Option<String>,
    pub post_checkpoint_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Resolved {
    pub settings: Settings,
    pub event_hooks: Vec<CallHook>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("missing Ovalix guardrail settings: {}", .missing.join(", "))]
pub struct OvalixConfigError {
    pub missing: Vec<&'static str>,
}

struct Candidates {
    tracker_api_base: Option<String>,
    tracker_api_key: Option<SecretString>,
    application_id: Option<String>,
    pre_checkpoint_id: Option<String>,
    post_checkpoint_id: Option<String>,
}

impl Candidates {
    fn from_params<E>(params: &Params, env: &E) -> Self
    where
        E: EnvironmentLookup,
    {
        Self {
            tracker_api_base: resolve_value(
                params.tracker_api_base.as_deref(),
                env,
                TRACKER_API_BASE_ENV,
            ),
            tracker_api_key: resolve_secret(
                params.tracker_api_key.as_ref(),
                env,
                TRACKER_API_KEY_ENV,
            ),
            application_id: resolve_value(
                params.application_id.as_deref(),
                env,
                APPLICATION_ID_ENV,
            ),
            pre_checkpoint_id: resolve_value(
                params.pre_checkpoint_id.as_deref(),
                env,
                PRE_CHECKPOINT_ID_ENV,
            ),
            post_checkpoint_id: resolve_value(
                params.post_checkpoint_id.as_deref(),
                env,
                POST_CHECKPOINT_ID_ENV,
            ),
        }
    }

    fn into_settings(self) -> Option<Settings> {
        Some(Settings {
            tracker_api_base: self.tracker_api_base?,
            tracker_api_key: self.tracker_api_key?,
            application_id: self.application_id?,
            pre_checkpoint_id: self.pre_checkpoint_id,
            post_checkpoint_id: self.post_checkpoint_id,
        })
    }

    fn missing(&self, requested: &[CallHook]) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if self.tracker_api_base.is_none() {
            missing.push(TRACKER_API_BASE_SETTING);
        }
        if self.tracker_api_key.is_none() {
            missing.push(TRACKER_API_KEY_SETTING);
        }
        if self.application_id.is_none() {
            missing.push(APPLICATION_ID_SETTING);
        }
        if requested.contains(&CallHook::PreCall) && self.pre_checkpoint_id.is_none() {
            missing.push(PRE_CHECKPOINT_SETTING);
        }
        if requested.contains(&CallHook::PostCall) && self.post_checkpoint_id.is_none() {
            missing.push(POST_CHECKPOINT_SETTING);
        }
        if self.pre_checkpoint_id.is_none() && self.post_checkpoint_id.is_none() {
            missing.push(ANY_CHECKPOINT_SETTING);
        }
        missing
    }
}

pub fn resolve<E>(
    params: &Params,
    env: &E,
    requested: &[CallHook],
) -> Result<Resolved, OvalixConfigError>
where
    E: EnvironmentLookup,
{
    let candidates = Candidates::from_params(params, env);
    let missing = candidates.missing(requested);
    if !missing.is_empty() {
        return Err(OvalixConfigError { missing });
    }
    let event_hooks = resolve_event_hooks(&candidates, requested);
    let Some(settings) = candidates.into_settings() else {
        return Err(OvalixConfigError { missing });
    };
    Ok(Resolved {
        settings,
        event_hooks,
    })
}

// LiteLLM's `or` chain treats a blank config value as unset, so the environment variable is read next.
fn resolve_value<E>(explicit: Option<&str>, env: &E, name: &str) -> Option<String>
where
    E: EnvironmentLookup,
{
    explicit
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| env.lookup(name).filter(|value| !value.is_empty()))
}

fn resolve_secret<E>(explicit: Option<&SecretString>, env: &E, name: &str) -> Option<SecretString>
where
    E: EnvironmentLookup,
{
    explicit
        .map(secrecy::ExposeSecret::expose_secret)
        .filter(|value| !value.is_empty())
        .map(SecretString::from)
        .or_else(|| {
            env.lookup(name)
                .filter(|value| !value.is_empty())
                .map(SecretString::from)
        })
}

// a checkpoint id without its hook still runs, so the hook is added to whatever the config requested
fn resolve_event_hooks(candidates: &Candidates, requested: &[CallHook]) -> Vec<CallHook> {
    let mut hooks = requested.to_vec();
    if candidates.pre_checkpoint_id.is_some() && !hooks.contains(&CallHook::PreCall) {
        hooks.push(CallHook::PreCall);
    }
    if candidates.post_checkpoint_id.is_some() && !hooks.contains(&CallHook::PostCall) {
        hooks.push(CallHook::PostCall);
    }
    hooks
}

mod tests;
