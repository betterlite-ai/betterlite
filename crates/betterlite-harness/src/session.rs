pub mod output;
pub mod routing;
pub mod skills;
pub mod validation;

use std::path::Path;

use uuid::Uuid;

use crate::context::{SessionContext, SessionSettings, TurnOutcome};
use crate::session::routing::{ProxySettings, resolve_model_route};
use crate::session::skills::{SkillError, normalize_skills};
use crate::types::Harness;

#[derive(Clone, Default)]
pub struct SessionParameters {
    pub settings: SessionSettings,
    pub install: bool,
}

#[derive(Clone)]
pub struct SessionConfig<S> {
    pub harness: Harness,
    pub sandbox: S,
    pub parameters: SessionParameters,
}

pub fn build_config<S>(
    harness: Harness,
    sandbox: S,
    mut parameters: SessionParameters,
    proxy: &ProxySettings,
    directory: &Path,
) -> Result<SessionConfig<S>, BuildError> {
    let settings = &mut parameters.settings;
    let (model, gateway) = resolve_model_route(
        settings.model.as_deref(),
        settings.api_key.as_ref(),
        settings.api_base.as_deref(),
        proxy,
    )?;
    settings.model = model;
    settings.gateway = gateway;
    settings.skills = normalize_skills(&settings.skills, directory)?;
    Ok(SessionConfig {
        harness,
        sandbox,
        parameters,
    })
}

#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error("{0}")]
    Route(#[from] routing::RouteError),
    #[error("{0}")]
    Skill(#[from] SkillError),
}

#[must_use]
pub fn context_for<S>(config: &SessionConfig<S>, session_id: Uuid) -> SessionContext<S>
where
    S: Clone,
{
    let settings = config.parameters.settings.clone();
    SessionContext {
        harness: config.harness,
        sandbox: config.sandbox.clone(),
        session_id: session_id.as_simple().to_string(),
        settings,
        outcome: TurnOutcome::default(),
    }
}
