use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::agent_host_settings::Settings;
use betterlite_openai::agent_and_diffusion_hosts::AgentHost;
use betterlite_openai::agent_and_diffusion_hosts::gradient::GradientAiConfig;
use betterlite_openai::agent_and_diffusion_hosts::heroku::HerokuChatConfig;
use betterlite_openai::agent_and_diffusion_hosts::inception_fim::{
    self, InceptionTextCompletionConfig,
};
use betterlite_openai::chat::ChatMessage;
use betterlite_openai::chat_content::Mode;
use betterlite_openai::openai_base_config::Object;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    scenarios: Vec<Scenario>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Scenario {
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    inception_key: Option<String>,
    #[serde(default)]
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Call {
    Info {
        host: AgentHost,
        #[serde(default)]
        api_base: Option<String>,
        #[serde(default)]
        api_key: Option<String>,
    },
    Supported {
        host: AgentHost,
    },
    GradientUrl {
        #[serde(default)]
        api_base: Option<String>,
    },
    GradientValidate {
        #[serde(default)]
        api_key: Option<String>,
        #[serde(default)]
        headers: Option<BTreeMap<String, String>>,
    },
    GradientParams {
        params: IndexMap<String, Value>,
        #[serde(default)]
        drop_params: bool,
    },
    HerokuUrl {
        #[serde(default)]
        api_base: Option<String>,
        #[serde(default)]
        api_key: Option<String>,
    },
    HerokuRequest {
        model: String,
        messages: Vec<ChatMessage>,
        #[serde(default)]
        optional: IndexMap<String, Value>,
        #[serde(default)]
        async_mode: bool,
    },
    FimSupported,
    FimParams {
        params: IndexMap<String, Value>,
    },
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut scenarios = Vec::with_capacity(input.scenarios.len());
        for scenario in &input.scenarios {
            scenarios.push(scenario_value(scenario)?);
        }
        Ok(json!(scenarios))
    }
}

fn scenario_value(scenario: &Scenario) -> HelperResult {
    let source = scenario
        .inception_key
        .as_ref()
        .map_or_else(String::new, |key| {
            json!({ "litellm_settings": { "inception_key": key } }).to_string()
        });
    let settings = ConfigLoader {
        lookup: scenario.environment.clone(),
    }
    .load(&source)
    .map_err(|error| helper_error(error.to_string()))?
    .sections
    .agent_host_settings;
    let steps = scenario
        .calls
        .iter()
        .map(|call| step_value(call, &settings))
        .collect::<Vec<_>>();
    Ok(json!({ "steps": steps }))
}

fn helper_error(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("ConfigError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn projection(result: Result<Value, Option<u16>>) -> Value {
    match result {
        Ok(value) => json!({ "ok": true, "value": value }),
        Err(status) => json!({ "ok": false, "error": { "status": status } }),
    }
}

fn step_value(call: &Call, host_settings: &Settings) -> Value {
    match call {
        Call::Info {
            host,
            api_base,
            api_key,
        } => info_value(
            *host,
            api_base.as_deref(),
            api_key.as_deref(),
            host_settings,
        ),
        Call::Supported { host } => {
            projection(Ok(json!(host.supported_params().unwrap_or_default())))
        }
        Call::GradientUrl { api_base } => gradient_url_value(api_base.as_deref(), host_settings),
        Call::GradientValidate { api_key, headers } => {
            gradient_validate_value(api_key.as_deref(), headers.clone(), host_settings)
        }
        Call::GradientParams {
            params,
            drop_params,
        } => gradient_params_value(params, *drop_params),
        Call::HerokuUrl { api_base, api_key } => {
            heroku_url_value(api_base.as_deref(), api_key.as_deref(), host_settings)
        }
        Call::HerokuRequest {
            model,
            messages,
            optional,
            async_mode,
        } => heroku_request_value(model, messages, optional, *async_mode),
        Call::FimSupported => projection(Ok(json!(inception_fim::SUPPORTED_PARAMS))),
        Call::FimParams { params } => {
            let mut mapped = Object::new();
            InceptionTextCompletionConfig::map_params(params, &mut mapped);
            projection(Ok(json!(mapped)))
        }
    }
}

fn gradient_url_value(api_base: Option<&str>, host_settings: &Settings) -> Value {
    projection(Ok(json!(GradientAiConfig::get_complete_url(
        host_settings,
        api_base
    ))))
}

fn info_value(
    host: AgentHost,
    api_base: Option<&str>,
    api_key: Option<&str>,
    host_settings: &Settings,
) -> Value {
    let key = api_key.map(SecretString::from);
    let (base, resolved) = host.provider_info(host_settings, api_base, key.as_ref());
    let resolved_key = resolved
        .as_ref()
        .map(|secret| secret.expose_secret().to_owned());
    projection(Ok(json!({ "base": base, "key": resolved_key })))
}

fn gradient_validate_value(
    api_key: Option<&str>,
    headers: Option<BTreeMap<String, String>>,
    host_settings: &Settings,
) -> Value {
    let key = api_key.map(SecretString::from);
    let validated = GradientAiConfig::validate_environment(host_settings, key.as_ref(), headers);
    projection(
        validated
            .map(|checked| json!(checked))
            .map_err(|error| error.status_code()),
    )
}

fn heroku_url_value(
    api_base: Option<&str>,
    api_key: Option<&str>,
    host_settings: &Settings,
) -> Value {
    let key = api_key.map(SecretString::from);
    match HerokuChatConfig::get_complete_url(host_settings, api_base, key.as_ref()) {
        Ok(url) => projection(Ok(json!(url))),
        Err(error) => projection(Err(error.status_code())),
    }
}

fn gradient_params_value(params: &IndexMap<String, Value>, drop_params: bool) -> Value {
    let mut mapped = Object::new();
    projection(
        GradientAiConfig::map_params(params, &mut mapped, drop_params)
            .map(|()| json!(mapped))
            .map_err(|error| error.status_code()),
    )
}

fn heroku_request_value(
    model: &str,
    messages: &[ChatMessage],
    optional: &IndexMap<String, Value>,
    async_mode: bool,
) -> Value {
    let chat_params: Object = optional
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let mode = if async_mode {
        Mode::AsyncSubclass
    } else {
        Mode::Sync
    };
    projection(
        HerokuChatConfig::transform_request(model, messages, &chat_params, mode)
            .map(|body| json!(body))
            .map_err(|error| error.status_code()),
    )
}

mod tests;
