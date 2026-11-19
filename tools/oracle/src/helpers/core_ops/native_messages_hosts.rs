mod tests;

use std::collections::BTreeMap;
use std::future::{Future, ready};

use betterlite_anthropic::anthropic_capability_catalog::OptionalParams;
use betterlite_anthropic::anthropic_capability_catalog::thinking::ThinkingError;
use betterlite_anthropic::capabilities::reasoning::ReasoningEffortError;
use betterlite_anthropic::credentials::{ANTHROPIC_VERSION, AnthropicSettings};
use betterlite_config::sections::anthropic_credentials::Settings as CredentialSettings;
use betterlite_config::sections::native_messages_hosts_settings::{
    HostEnvironment, Settings as HostSettings,
};
use betterlite_core::api::messages_native_transform::capabilities::{
    LegacyShapes, Thinking, ThinkingCapabilities,
};
use betterlite_core::native_messages_hosts::deepseek;
use betterlite_core::native_messages_hosts::error::HostError;
use betterlite_core::native_messages_hosts::keys::BaseSources;
use betterlite_core::native_messages_hosts::minimax;
use betterlite_core::native_messages_hosts::request::{
    EnvironmentRequest, HostRequest, validate_inherited_environment,
};
use betterlite_core::native_messages_hosts::tencent;
use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Number, Value, json};
use tokio::runtime::{Builder, Runtime};

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
enum Step {
    Url(UrlCall),
    Environment(EnvironmentCall),
    Transform(TransformCall),
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Host {
    Deepseek,
    Tencent,
    Minimax,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UrlCall {
    host: Host,
    api_bases: Vec<Option<String>>,
    environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EnvironmentCall {
    host: Host,
    api_base: Option<String>,
    headers: IndexMap<String, String>,
    optional_params: IndexMap<String, Json>,
    messages: Vec<Json>,
    explicit_api_key: Option<String>,
    global_api_key: Option<String>,
    environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TransformCall {
    host: Host,
    model: String,
    messages: Vec<Json>,
    max_tokens: i64,
    system: Option<Json>,
    tools: Option<Json>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .build()
            .map_err(|source| invalid(source.to_string()))?;
        let mut answers = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            answers.push(answer(call, &runtime)?);
        }
        Ok(json!({ "calls": answers }))
    }
}

fn answer(step: &Step, runtime: &Runtime) -> HelperResult {
    match step {
        Step::Url(call) => Ok(url(call)),
        Step::Environment(call) => environment(call),
        Step::Transform(call) => transform(call, runtime),
    }
}

const fn environment_of(host: Host, settings: &HostSettings) -> &HostEnvironment {
    match host {
        Host::Deepseek => &settings.deepseek,
        Host::Tencent => &settings.tencent,
        Host::Minimax => &settings.minimax,
    }
}

fn url(call: &UrlCall) -> Value {
    let settings = HostSettings::resolve_environment(&call.environment);
    let host = environment_of(call.host, &settings);
    let urls: Vec<String> = call
        .api_bases
        .iter()
        .map(|api_base| {
            let sources = BaseSources {
                explicit: api_base.as_deref(),
                environment: host.api_base.as_deref(),
                fallback_environment: host.fallback_api_base.as_deref(),
            };
            match call.host {
                Host::Deepseek => deepseek::complete_url(sources),
                Host::Tencent => tencent::complete_url(sources),
                Host::Minimax => minimax::complete_url(sources),
            }
        })
        .collect();
    json!({ "urls": urls })
}

fn environment(call: &EnvironmentCall) -> HelperResult {
    let settings = HostSettings::resolve_environment(&call.environment);
    let host = environment_of(call.host, &settings);
    let explicit = call
        .explicit_api_key
        .as_deref()
        .map(|key| SecretString::from(key.to_owned()));
    let global = call
        .global_api_key
        .as_deref()
        .map(|key| SecretString::from(key.to_owned()));
    let mut headers = call.headers.clone();
    let request = EnvironmentRequest {
        headers: &mut headers,
        explicit_key: explicit.as_ref(),
        environment_key: host.api_key.as_ref(),
        global_key: global.as_ref(),
        api_base: call.api_base.as_deref(),
        optional_params: &call.optional_params,
        messages: &call.messages,
    };
    let api_base = match call.host {
        Host::Deepseek => deepseek::validate_environment(request),
        Host::Tencent | Host::Minimax => {
            validate_inherited_environment(request, &credentials(&call.environment)?)
        }
    }
    .map_err(|error| host_failure(&error))?;
    Ok(json!({ "headers": headers, "api_base": api_base }))
}

fn transform(call: &TransformCall, runtime: &Runtime) -> HelperResult {
    let mut parameters = Object::new();
    parameters.insert(
        "max_tokens".to_owned(),
        Json::Number(Number::from(call.max_tokens)),
    );
    if let Some(system) = &call.system {
        parameters.insert("system".to_owned(), system.clone());
    }
    if let Some(tools) = &call.tools {
        parameters.insert("tools".to_owned(), tools.clone());
    }
    let request = HostRequest {
        model: &call.model,
        messages: call.messages.clone(),
        optional_params: parameters,
    };
    let mut capabilities = Plain;
    let body = runtime
        .block_on(transform_for(call.host, &mut capabilities, request))
        .map_err(|error| host_failure(&error))?;
    Ok(json!({
        "system": body.get("system"),
        "messages": body.get("messages"),
        "tools": body.get("tools"),
    }))
}

async fn transform_for<Capabilities>(
    host: Host,
    capabilities: &mut Capabilities,
    request: HostRequest<'_>,
) -> Result<Object, HostError>
where
    Capabilities: ThinkingCapabilities + Send,
{
    match host {
        Host::Deepseek => deepseek::transform_request(capabilities, request).await,
        Host::Tencent => tencent::transform_request(capabilities, request).await,
        Host::Minimax => minimax::transform_request(capabilities, request).await,
    }
}

fn credentials(
    environment: &BTreeMap<String, String>,
) -> Result<AnthropicSettings, Box<HelperError>> {
    let resolved = CredentialSettings::resolve_environment(environment)
        .map_err(|error| invalid(error.to_string()))?;
    Ok(AnthropicSettings {
        api_key: resolved.api_key,
        auth_token: resolved.auth_token,
        api_base: resolved.api_base,
        use_bearer_for_custom_base: resolved.use_bearer_for_custom_base,
        anthropic_version: ANTHROPIC_VERSION.to_owned(),
    })
}

fn host_failure(error: &HostError) -> Box<HelperError> {
    Box::new(HelperError {
        class: error.class_name().to_owned(),
        status: error.status_code(),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn invalid(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

// The cases use no thinking params, so the transform's capability questions all answer no.
struct Plain;

impl ThinkingCapabilities for Plain {
    fn supports(
        &mut self,
        _model: &str,
        _key: &str,
        _provider: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn map_effort<'seam>(
        &'seam mut self,
        _model: &'seam str,
        _effort: &'seam str,
        _provider: &'seam str,
    ) -> impl Future<Output = Result<Option<Thinking>, ReasoningEffortError>> + Send + 'seam {
        ready(Ok(None))
    }

    fn validate_effort(
        &mut self,
        _model: &str,
        _effort: Option<&str>,
        _provider: &str,
    ) -> impl Future<Output = Option<String>> + Send {
        ready(None)
    }

    fn supports_effort_param(
        &mut self,
        _model: &str,
        _provider: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn drop_disabled_thinking(
        &mut self,
        _model: &str,
        _params: &mut OptionalParams,
        _provider: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn translate_legacy_thinking<'seam>(
        &'seam mut self,
        _request: &'seam mut LegacyShapes<'seam>,
    ) -> impl Future<Output = Result<(), ThinkingError>> + Send + 'seam {
        ready(Ok(()))
    }
}
