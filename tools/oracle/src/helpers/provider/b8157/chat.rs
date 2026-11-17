use std::collections::BTreeMap;

use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::provider_defaults_b::special::{SPECIAL_TABLE, resolve_special};
use betterlite_openai::compatible_wire::{EndpointType, header_map, validate_environment};
use betterlite_openai::hosted_vllm::chat::provider_info;
use betterlite_openai::local_servers::ProviderEnvironment;
use betterlite_openai::openai_base_config::Headers;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_prompts::arguments::json::Object;
use betterlite_provider::ollama_chat::config as ollama_chat_config;
use betterlite_provider::ollama_generate::config as ollama_generate_config;
use betterlite_provider::self_hosted_dispatch::call::CompletionCall;
use betterlite_provider::self_hosted_dispatch::chat::{complete_lemonade, dispatch};
use betterlite_provider::self_hosted_dispatch::context::{
    CallLogging, DispatchContext, DispatchSettings,
};
use betterlite_provider::self_hosted_dispatch::error::DispatchError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::json;

use super::fixtures::{ORACLE_UPSTREAM, Recorder, environment, logging, section_settings};
use super::input::{Case, Globals, Provider};
use super::input_failure;
use super::wire::{error_or_null, kept_authorization};

const HOSTED_VLLM_API_BASE: &str = "HOSTED_VLLM_API_BASE";
const HOSTED_VLLM_API_KEY: &str = "HOSTED_VLLM_API_KEY";
const OLLAMA_API_BASE: &str = "OLLAMA_API_BASE";
const OLLAMA_API_KEY: &str = "OLLAMA_API_KEY";
const OLLAMA_DEFAULT_API_BASE: &str = "http://localhost:11434";

// what get_llm_provider settles before main.py hands the call to an arm
struct PreResolved {
    api_base: Option<String>,
    api_key: Option<SecretString>,
}

pub(super) fn run(case: &Case) -> HelperResult {
    let env = environment(case);
    let settings = section_settings(case, &env);
    let empty = Object::new();
    let optional_params = case.optional_params()?;
    let passed_key = case.api_key.as_deref().map(SecretString::from);
    let passed_base = case
        .api_base
        .as_deref()
        .map(|value| value.replace("{{upstream}}", ORACLE_UPSTREAM));
    let resolved = pre_resolved(
        case.provider,
        passed_base.as_deref(),
        passed_key.as_ref(),
        &case.globals,
        &env,
    )?;
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_value_error()?;
    let (mut state, prepost) = logging(&redactor)?;
    let handler = Recorder::for_case(case);
    let context = DispatchContext {
        acompletion: false,
        api_base: resolved.api_base.as_deref(),
        api_key: resolved.api_key.as_ref(),
        custom_llm_provider: case.provider.slug(),
        headers: case.headers.as_ref(),
        litellm_params: &empty,
        messages: &case.messages,
        model: &case.model,
        optional_params: &optional_params,
        stream: case.stream,
        timeout: None,
        settings: DispatchSettings::from(&settings),
    };
    let mut log = CallLogging {
        state: &mut state,
        env: &prepost,
    };
    let outcome = if matches!(case.provider, Provider::Lemonade) {
        complete_lemonade(&context, &handler)
    } else {
        let tag = case
            .provider
            .chat_tag()
            .ok_or_else(|| input_failure(format!("{} has no chat arm", case.provider.slug())))?;
        dispatch(tag, &context, &handler, &mut log)
            .ok_or_else(|| input_failure(String::from("the chat table has no arm")))?
    };
    let call = handler.first()?;
    let (url, headers) = chat_request(case.provider, &call)?;
    let view = json!({
        "url": url,
        "headers": kept_authorization(&header_map(&headers).or_value_error()?),
    });
    Ok(json!({"request": view, "error": error_or_null(&outcome)}))
}

fn pre_resolved(
    provider: Provider,
    passed_base: Option<&str>,
    passed_key: Option<&SecretString>,
    globals: &Globals,
    env: &BTreeMap<String, String>,
) -> Result<PreResolved, Box<HelperError>> {
    match provider {
        Provider::HostedVllm => {
            let environment = ProviderEnvironment {
                api_base: env.get(HOSTED_VLLM_API_BASE).cloned(),
                api_key: env
                    .get(HOSTED_VLLM_API_KEY)
                    .cloned()
                    .map(SecretString::from),
            };
            let info = provider_info(passed_base, passed_key, &environment);
            Ok(PreResolved {
                api_base: info.api_base,
                api_key: Some(info.api_key),
            })
        }
        Provider::Ollama => {
            let api_base = passed_base
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .or_else(|| {
                    env.get(OLLAMA_API_BASE)
                        .filter(|value| !value.is_empty())
                        .cloned()
                })
                .unwrap_or_else(|| String::from(OLLAMA_DEFAULT_API_BASE));
            let api_key = passed_key
                .filter(|key| !key.expose_secret().is_empty())
                .cloned()
                .or_else(|| {
                    env.get(OLLAMA_API_KEY)
                        .filter(|value| !value.is_empty())
                        .map(|value| SecretString::from(value.as_str()))
                });
            Ok(PreResolved {
                api_base: Some(api_base),
                api_key,
            })
        }
        Provider::Lemonade => {
            let defaults = SPECIAL_TABLE
                .iter()
                .find(|(id, _)| *id == ProviderId::Lemonade)
                .map(|(_, defaults)| defaults)
                .ok_or_else(|| {
                    input_failure(String::from("the lemonade provider defaults are missing"))
                })?;
            let credentials = GlobalCredentials {
                api_key: None,
                api_base: None,
                xai_key: None,
                inception_key: None,
                lemonade_key: globals.lemonade_key.as_deref().map(SecretString::from),
            };
            let key = passed_key
                .filter(|key| !key.expose_secret().is_empty())
                .map(|key| key.expose_secret());
            let (api_base, api_key) =
                resolve_special(passed_base, key, defaults, &credentials, env);
            Ok(PreResolved {
                api_base,
                api_key: api_key.map(SecretString::from),
            })
        }
        Provider::OllamaChat | Provider::Petals => Ok(PreResolved {
            api_base: passed_base
                .filter(|value| !value.is_empty())
                .map(str::to_owned),
            api_key: passed_key
                .filter(|key| !key.expose_secret().is_empty())
                .cloned(),
        }),
        Provider::Triton | Provider::Oobabooga | Provider::Xinference => Err(input_failure(
            format!("{provider:?} has no chat request to record"),
        )),
    }
}

fn chat_request(
    provider: Provider,
    call: &CompletionCall,
) -> Result<(String, Headers), Box<HelperError>> {
    match provider {
        Provider::HostedVllm | Provider::Lemonade => Ok(validate_environment(
            call.api_key.as_ref(),
            call.api_base.as_deref(),
            EndpointType::ChatCompletions,
            call.headers.clone(),
            false,
        )
        .or_value_error()?),
        Provider::Ollama => Ok((
            ollama_generate_config::complete_url(call.api_base.as_deref()),
            call.headers.clone().unwrap_or_default(),
        )),
        // the arm put the bearer header on the call itself, and OllamaChatConfig only fills an absent one
        Provider::OllamaChat => Ok((
            ollama_chat_config::complete_url(call.api_base.as_deref()),
            call.headers.clone().unwrap_or_default(),
        )),
        Provider::Petals => match call.api_base.clone() {
            Some(api_base) => Ok((api_base, call.headers.clone().unwrap_or_default())),
            None => Err(input_failure(String::from(
                "the petals arm called no handler without an api_base",
            ))),
        },
        Provider::Triton | Provider::Oobabooga | Provider::Xinference => Err(input_failure(
            format!("{provider:?} has no chat request to record"),
        )),
    }
}
