use betterlite_core::agents::AgentResponse;
use betterlite_core::agents::json::AgentJson;
use betterlite_core::agents::kill_switch::{AgentKillSwitchAuth, AgentKillSwitchConfig};
use betterlite_redact::structured::SensitiveDataMasker;

use crate::agent_credentials::REDACTED_BY_LITELM;
use crate::agent_credentials::error::AgentCredentialsError;
// ast-grep-ignore: no-long-names
use crate::agent_credentials::redact::redact_agent_litellm_params_map as redact_params_map;
use crate::kill_switch_management::redact::redact_kill_switch;

pub struct JwtAgentSettings {
    pub enabled: bool,
    pub agent_id_field_set: bool,
}

pub fn redact_sensitive_agent_fields(
    agents: &[AgentResponse],
    is_admin: bool,
    jwt: JwtAgentSettings,
    masker: &SensitiveDataMasker,
) -> Result<Vec<AgentJson>, AgentCredentialsError> {
    let mut redacted = Vec::with_capacity(agents.len());
    for agent in agents {
        let mut copy = agent.clone();
        copy.jwt_auth_configured =
            jwt.enabled && (agent.identity.is_some() || jwt.agent_id_field_set);
        if !is_admin {
            copy.static_headers = None;
            copy.extra_headers = None;
            copy.keys = None;
            copy.kill_switch = None;
            copy.identity = None;
        }
        if let Some(params) = copy
            .litellm_params
            .as_ref()
            .filter(|params| !params.is_empty())
        {
            copy.litellm_params = Some(redact_params_map(params, masker));
        }
        copy.kill_switch = redact_kill_switch(copy.kill_switch.as_ref());
        let mut value = serde_json::to_value(&copy).map_err(AgentCredentialsError::Serialize)?;
        if let Some(config) = copy.kill_switch.as_ref() {
            write_kill_switch_marker(&mut value, config);
        }
        redacted.push(value);
    }
    Ok(redacted)
}

fn write_kill_switch_marker(value: &mut AgentJson, config: &AgentKillSwitchConfig) {
    let Some(auth) = config.auth.as_ref() else {
        return;
    };
    let Some(auth_fields) = value
        .get_mut("kill_switch")
        .and_then(|kill_switch| kill_switch.get_mut("auth"))
        .and_then(AgentJson::as_object_mut)
    else {
        return;
    };
    let field = match auth {
        AgentKillSwitchAuth::Bearer { .. } => "token",
        AgentKillSwitchAuth::ApiKey { .. } => "api_key",
        AgentKillSwitchAuth::Basic { .. } => "password",
    };
    auth_fields.insert(
        String::from(field),
        AgentJson::String(String::from(REDACTED_BY_LITELM)),
    );
}

mod tests;
