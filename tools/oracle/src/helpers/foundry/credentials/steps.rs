use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::foundry_credentials::Settings;
use betterlite_foundry::credentials::AZURE_AI_API_KEY_VAR;
use betterlite_foundry::credentials::agent::{
    AgentTokenRequest, get_azure_ai_agent_entra_token, has_azure_entra_params,
    resolve_agent_auth_header,
};
use betterlite_foundry::credentials::auth_headers::{AuthHeaderRequest, get_azure_ai_auth_headers};
use betterlite_foundry::credentials::entra::{EntraTokenRequest, get_azure_ai_entra_token};
use betterlite_foundry::credentials::error::CredentialsError;
use betterlite_foundry::credentials::host::{
    ApiKeyHeader, api_key_header_for_base, is_azure_openai_v1_host, is_foundry_base,
};
use betterlite_foundry::credentials::model_info::ModelInfoAccessors;
use betterlite_foundry::credentials::native::{foundry_rejects_tools, supports_native_responses};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use crate::helpers::{HelperError, HelperResult, OrHelperError as _};

use super::fixture::{EchoSources, secrets};
use super::{AgentStep, AuthStep, GateStep, HeaderName, HostStep, Input, NativeStep, SettingsStep};

#[derive(Clone, Copy)]
enum AgentOutput {
    Headers,
    Token,
}

pub(super) fn run(input: &Input) -> HelperResult {
    let steps = match input {
        Input::AgentHeaders(steps) => steps
            .iter()
            .map(|step| agent_step(step, AgentOutput::Headers))
            .collect(),
        Input::AgentTokens(steps) => steps
            .iter()
            .map(|step| agent_step(step, AgentOutput::Token))
            .collect(),
        Input::AuthHeaders(steps) => steps.iter().map(auth_step).collect(),
        Input::HasEntra(steps) => steps
            .iter()
            .map(|step| json!(has_azure_entra_params(step.params.as_ref())))
            .collect(),
        Input::HostBoundary(steps) => steps.iter().map(host_step).collect(),
        Input::NativeResponses(steps) => steps
            .iter()
            .map(native_step)
            .collect::<Result<Vec<_>, _>>()?,
        Input::ReasoningGate(steps) => steps.iter().map(gate_step).collect(),
        Input::Settings(steps) => steps
            .iter()
            .map(settings_step)
            .collect::<Result<Vec<_>, _>>()?,
    };
    Ok(json!({ "steps": steps }))
}

fn foundry_settings(environment: &BTreeMap<String, String>) -> Result<Settings, Box<HelperError>> {
    let config = ConfigLoader {
        lookup: environment.clone(),
    }
    .load("")
    .or_config_error()?;
    Ok(config.sections.foundry_credentials)
}

fn host_step(step: &HostStep) -> Value {
    json!({
        "foundry_base": is_foundry_base(&step.api_base),
        "header": api_key_header_for_base(Some(&step.api_base)).wire_name(),
        "v1_host": is_azure_openai_v1_host(Some(&step.api_base)),
    })
}

fn agent_step(step: &AgentStep, output: AgentOutput) -> Value {
    let sources = EchoSources;
    let secrets = secrets();
    let request = AgentTokenRequest {
        params: &step.params,
        environment: &step.environment,
        secrets: &secrets,
        sources: &sources,
    };
    let reply: Result<Value, CredentialsError> = match output {
        AgentOutput::Headers => resolve_agent_auth_header(&request)
            .map(|headers| json!({"kind": "headers", "headers": headers})),
        AgentOutput::Token => get_azure_ai_agent_entra_token(&request)
            .map(|token| json!({"kind": "token", "token": token.expose_secret()})),
    };
    reply.unwrap_or_else(|_| json!({"kind": "error"}))
}

fn auth_step(step: &AuthStep) -> Value {
    let sources = EchoSources;
    let header = match step.header {
        HeaderName::Authorization => ApiKeyHeader::Authorization,
        HeaderName::ApiKey => ApiKeyHeader::ApiKey,
        HeaderName::ApiKeyTitleCase => ApiKeyHeader::ApiKeyTitleCase,
        HeaderName::OcpApimSubscriptionKey => ApiKeyHeader::OcpApimSubscriptionKey,
    };
    let api_key = step.api_key.clone().map(SecretString::from);
    let request = AuthHeaderRequest {
        api_key: api_key.as_ref(),
        params: step.params.as_ref(),
        environment: &step.environment,
        api_key_header: header,
        api_key_env_var: step
            .api_key_env_var
            .as_deref()
            .unwrap_or(AZURE_AI_API_KEY_VAR),
        refresh_enabled: false,
        sources: &sources,
    };
    get_azure_ai_auth_headers(&request).map_or_else(
        |_| json!({"kind": "error"}),
        |headers| json!({"kind": "headers", "headers": headers}),
    )
}

fn gate_step(step: &GateStep) -> Value {
    json!(foundry_rejects_tools(
        &step.model,
        step.reasoning_effort.as_ref(),
    ))
}

fn native_step(step: &NativeStep) -> Result<Value, Box<HelperError>> {
    let settings = foundry_settings(&step.environment)?;
    let accessors = ModelInfoAccessors {
        global_api_base: step.global_api_base.as_deref(),
        global_api_key: None,
        environment_api_base: settings.api_base.as_deref(),
        environment_api_key: settings.api_key.as_ref(),
    };
    Ok(json!(supports_native_responses(
        step.model.as_deref(),
        step.api_base.as_deref(),
        accessors,
    )))
}

fn settings_step(step: &SettingsStep) -> Result<Value, Box<HelperError>> {
    let settings = foundry_settings(&step.environment)?;
    let global_api_key = step.global_api_key.clone().map(SecretString::from);
    let explicit_api_key = step.api_key.clone().map(SecretString::from);
    let accessors = ModelInfoAccessors {
        global_api_base: step.global_api_base.as_deref(),
        global_api_key: global_api_key.as_ref(),
        environment_api_base: settings.api_base.as_deref(),
        environment_api_key: settings.api_key.as_ref(),
    };
    let sources = EchoSources;
    let reply = get_azure_ai_entra_token(&EntraTokenRequest {
        params: step.params.as_ref(),
        environment: &step.environment,
        refresh_enabled: false,
        sources: &sources,
    });
    let outcome = match reply {
        Ok(Some(token)) => json!({"kind": "token", "token": token.expose_secret()}),
        Ok(None) => json!({"kind": "absent"}),
        Err(_) => json!({"kind": "error"}),
    };
    let api_key = accessors
        .api_key(explicit_api_key.as_ref())
        .map(|key| key.expose_secret().to_owned());
    Ok(json!({
        "api_base": accessors.api_base(step.api_base.as_deref()),
        "api_key": api_key,
        "outcome": outcome,
    }))
}
