use std::env;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use secrecy::SecretString;
use serde::de::DeserializeOwned;
use uuid::Uuid;

use crate::context::{GatewayTarget, SessionContext, SessionSettings, TurnOutcome};
use crate::errors::HarnessError;
use crate::options::{HarnessOptions, OptionsKind};
use crate::payloads::{SKILL_MANIFEST, last_json_object};
use crate::transformation::BaseHarnessConfig;
use crate::types::{Capabilities, Capability, Harness, JsonObject, PermissionMode};

pub const PERMISSION_MODES: &[PermissionMode] = &[
    PermissionMode::Full,
    PermissionMode::ReadOnly,
    PermissionMode::Ask,
    PermissionMode::Edit,
];
pub const LITELLM_PROXY_PREFIX: &str = "litellm_proxy/";

#[derive(Clone)]
pub struct SessionConfig<S> {
    pub harness: Harness,
    pub sandbox: S,
    pub model: Option<String>,
    pub gateway: Option<GatewayTarget>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub instructions: Option<String>,
    pub tools: Vec<String>,
    pub skills: Vec<String>,
    pub disable_tools: Vec<String>,
    pub permissions: PermissionMode,
    pub on_approval: Option<crate::context::ApprovalHandler>,
    pub output_schema: Option<JsonObject>,
    pub max_turns: Option<u32>,
    pub timeout: Option<Duration>,
    pub metadata: JsonObject,
    pub options: Option<HarnessOptions>,
    pub install: bool,
}

#[derive(Debug)]
pub enum SessionError {
    Invalid(String),
    Harness(HarnessError),
}

impl fmt::Display for SessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => formatter.write_str(message),
            Self::Harness(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for SessionError {}

impl From<HarnessError> for SessionError {
    fn from(error: HarnessError) -> Self {
        Self::Harness(error)
    }
}

pub fn resolve_model_route(
    model: Option<&str>,
    api_key: Option<&str>,
    api_base: Option<&str>,
    use_litellm_proxy: bool,
) -> Result<(Option<String>, Option<GatewayTarget>), SessionError> {
    let prefixed = model.is_some_and(|value| value.starts_with(LITELLM_PROXY_PREFIX));
    if !prefixed && !use_litellm_proxy {
        return Ok((model.map(str::to_owned), None));
    }
    let routed_model = model
        .filter(|_| prefixed)
        .map(|value| value[LITELLM_PROXY_PREFIX.len()..].to_owned())
        .or_else(|| model.map(str::to_owned));
    let base = api_base
        .map(str::to_owned)
        .or_else(|| env::var("LITELLM_PROXY_API_BASE").ok())
        .unwrap_or_default();
    let key = api_key
        .map(str::to_owned)
        .or_else(|| env::var("LITELLM_PROXY_API_KEY").ok())
        .unwrap_or_default();
    let base = base.trim().trim_end_matches('/').to_owned();
    let key = key.trim().to_owned();
    if base.is_empty() {
        return Err(SessionError::Invalid(
            "litellm_proxy/ models need the gateway URL: pass api_base= or set LITELLM_PROXY_API_BASE"
                .to_owned(),
        ));
    }
    if key.is_empty() {
        return Err(SessionError::Invalid(
            "litellm_proxy/ models need a gateway virtual key: pass api_key= or set LITELLM_PROXY_API_KEY"
                .to_owned(),
        ));
    }
    Ok((
        routed_model,
        Some(GatewayTarget {
            api_base: base,
            api_key: SecretString::from(key),
        }),
    ))
}

pub fn normalize_skill(skill: impl AsRef<Path>) -> Result<String, SessionError> {
    let path = absolute_path(skill.as_ref());
    if !path.join(SKILL_MANIFEST).is_file() {
        return Err(SessionError::Invalid(format!(
            "Skill folder {path:?} has no {SKILL_MANIFEST}"
        )));
    }
    Ok(path.to_string_lossy().into_owned())
}

pub fn normalize_skills(skills: &[PathBuf]) -> Result<Vec<String>, SessionError> {
    skills.iter().map(normalize_skill).collect()
}

pub use check_basic as _check_basic;
pub use check_capabilities as _check_capabilities;
pub use check_options as _check_options;
pub use context_for as _context_for;
pub use normalize_skill as _normalize_skill;
pub use normalize_skills as _normalize_skills;

fn absolute_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_owned();
    }
    env::current_dir().map_or_else(|_| path.to_owned(), |directory| directory.join(path))
}

pub fn check_basic<S>(config: &SessionConfig<S>) -> Result<(), SessionError> {
    if !PERMISSION_MODES.contains(&config.permissions) {
        return Err(SessionError::Invalid(format!(
            "permissions must be one of {PERMISSION_MODES:?}, got {:?}",
            config.permissions
        )));
    }
    if config.max_turns.is_some_and(|turns| turns == 0) {
        return Err(SessionError::Invalid("max_turns must be >= 1".to_owned()));
    }
    if config.timeout.is_some_and(|timeout| timeout.is_zero()) {
        return Err(SessionError::Invalid("timeout must be > 0".to_owned()));
    }
    if config.install {
        return Err(SessionError::Invalid(
            "install=True is not supported yet; put the runtime binary on PATH in the sandbox"
                .to_owned(),
        ));
    }
    Ok(())
}

pub fn check_options<S, C: BaseHarnessConfig>(
    config: &SessionConfig<S>,
    _harness_config: &C,
) -> Result<(), SessionError> {
    if config.options.is_none() {
        return Ok(());
    }
    C::Options::pick(
        config
            .options
            .as_ref()
            .ok_or_else(|| SessionError::Invalid("missing options".to_owned()))?,
    )
    .map(|_| ())
    .ok_or_else(|| {
        HarnessError::OptionsMismatch {
            given: config
                .options
                .as_ref()
                .map_or("unknown", HarnessOptions::name),
            harness: config.harness,
            expected: C::Options::NAME,
        }
        .into()
    })
}

pub fn check_capabilities<S>(
    config: &SessionConfig<S>,
    capabilities: &Capabilities,
    interactive: bool,
) -> Result<(), SessionError> {
    if !capabilities.permission_modes.contains(&config.permissions) {
        return unsupported(config.harness, "permission mode");
    }
    if config.permissions == PermissionMode::Ask {
        if !capabilities.supported.contains(&Capability::ToolApproval) {
            return unsupported(config.harness, "tool approvals");
        }
        if config.on_approval.is_none() && !interactive {
            return Err(SessionError::Invalid(
                "permissions='ask' needs on_approval, or use stream() and answer Approval events"
                    .to_owned(),
            ));
        }
    }
    if config.output_schema.is_some()
        && !capabilities
            .supported
            .contains(&Capability::StructuredOutput)
    {
        return unsupported(config.harness, "output");
    }
    if !config.tools.is_empty() && !capabilities.supported.contains(&Capability::CustomTools) {
        return unsupported(config.harness, "custom tools");
    }
    if !config.skills.is_empty() && !capabilities.supported.contains(&Capability::Skills) {
        return unsupported(config.harness, "skills");
    }
    if !config.disable_tools.is_empty()
        && !capabilities.supported.contains(&Capability::ToolFiltering)
    {
        return unsupported(config.harness, "tool filtering");
    }
    Ok(())
}

fn unsupported(harness: Harness, capability: &'static str) -> Result<(), SessionError> {
    Err(SessionError::Invalid(format!(
        "Harness.{} does not support {capability}",
        harness.name()
    )))
}

pub fn validate<S, C: BaseHarnessConfig>(
    config: &SessionConfig<S>,
    harness_config: &C,
    interactive: bool,
) -> Result<(), SessionError> {
    check_basic(config)?;
    check_options(config, harness_config)?;
    check_capabilities(config, &harness_config.capabilities(), interactive)
}

pub fn build_config<S>(
    harness: Harness,
    sandbox: S,
    model: Option<String>,
    api_key: Option<String>,
    api_base: Option<String>,
    instructions: Option<String>,
    tools: Vec<String>,
    skills: Vec<PathBuf>,
    disable_tools: Vec<String>,
    permissions: PermissionMode,
    on_approval: Option<crate::context::ApprovalHandler>,
    output_schema: Option<JsonObject>,
    max_turns: Option<u32>,
    timeout: Option<Duration>,
    metadata: Option<JsonObject>,
    options: Option<HarnessOptions>,
    install: bool,
    use_litellm_proxy: bool,
) -> Result<SessionConfig<S>, SessionError> {
    let (routed_model, gateway) = resolve_model_route(
        model.as_deref(),
        api_key.as_deref(),
        api_base.as_deref(),
        use_litellm_proxy,
    )?;
    Ok(SessionConfig {
        harness,
        sandbox,
        model: routed_model,
        gateway,
        api_key: api_key.map(SecretString::from),
        api_base,
        instructions,
        tools,
        skills: normalize_skills(&skills)?,
        disable_tools,
        permissions,
        on_approval,
        output_schema,
        max_turns,
        timeout,
        metadata: metadata.unwrap_or_default(),
        options,
        install,
    })
}

pub fn context_for<S>(config: &SessionConfig<S>) -> SessionContext<S>
where
    S: Clone,
{
    SessionContext {
        harness: config.harness,
        sandbox: config.sandbox.clone(),
        session_id: Uuid::new_v4().as_simple().to_string(),
        settings: SessionSettings {
            model: config.model.clone(),
            gateway: config.gateway.clone(),
            api_key: config.api_key.clone(),
            api_base: config.api_base.clone(),
            instructions: config.instructions.clone(),
            skills: config.skills.clone(),
            disable_tools: config.disable_tools.clone(),
            permissions: config.permissions,
            on_approval: config.on_approval.clone(),
            output_schema: config.output_schema.clone(),
            max_turns: config.max_turns,
            timeout: config.timeout,
            metadata: config.metadata.clone(),
            options: config.options.clone(),
        },
        outcome: TurnOutcome::default(),
    }
}

pub fn parse_output<T: DeserializeOwned>(
    output_json: Option<&str>,
    text: &str,
) -> (Option<T>, Option<String>) {
    let Some(raw) = output_json
        .map(str::to_owned)
        .or_else(|| last_json_object(text))
    else {
        return (
            None,
            Some("no JSON object found in the final answer".to_owned()),
        );
    };
    match serde_json::from_str(&raw) {
        Ok(parsed) => (Some(parsed), None),
        Err(error) => (None, Some(error.to_string())),
    }
}

#[cfg(test)]
mod session_input {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Deserialize, Eq, PartialEq)]
    struct Answer {
        value: u32,
    }

    #[test]
    fn parses_the_final_json_object() {
        let (answer, error) = parse_output::<Answer>(None, "progress\n{\"value\": 7}");
        assert_eq!(answer, Some(Answer { value: 7 }));
        assert!(error.is_none());
    }

    #[test]
    fn rejects_invalid_basic_settings() {
        let config = SessionConfig {
            harness: Harness::Codex,
            sandbox: (),
            model: None,
            gateway: None,
            api_key: None,
            api_base: None,
            instructions: None,
            tools: Vec::new(),
            skills: Vec::new(),
            disable_tools: Vec::new(),
            permissions: PermissionMode::Full,
            on_approval: None,
            output_schema: None,
            max_turns: Some(0),
            timeout: None,
            metadata: JsonObject::new(),
            options: None,
            install: false,
        };
        assert_eq!(
            check_basic(&config).unwrap_err().to_string(),
            "max_turns must be >= 1"
        );
    }

    #[test]
    fn strips_proxy_prefix_and_gateway_slash() {
        let (model, gateway) = resolve_model_route(
            Some("litellm_proxy/team"),
            Some(" key "),
            Some(" https://gateway/// "),
            false,
        )
        .unwrap();
        assert_eq!(model.as_deref(), Some("team"));
        assert_eq!(
            gateway.as_ref().map(|target| target.api_base.as_str()),
            Some("https://gateway")
        );
    }
}
