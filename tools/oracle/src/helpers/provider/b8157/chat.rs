use std::collections::BTreeMap;

use betterlite_logging::logging_pre_post::context::LogContext;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::provider_defaults_b::special::{
    SPECIAL_TABLE, SpecialDefaults, resolve_special,
};
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
use betterlite_provider::self_hosted_dispatch::context::{DispatchContext, DispatchSettings};
use betterlite_provider::self_hosted_dispatch::error::DispatchError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::json;

use super::fixtures::{ORACLE_UPSTREAM, Recorder, environment, logging, section_settings};
use super::input::{Case, Provider};
use super::input_failure;
use super::wire::{error_or_null, kept_authorization};

const HOSTED_VLLM_API_BASE: &str = "HOSTED_VLLM_API_BASE";
const HOSTED_VLLM_API_KEY: &str = "HOSTED_VLLM_API_KEY";
const OLLAMA_API_BASE: &str = "OLLAMA_API_BASE";
const OLLAMA_API_KEY: &str = "OLLAMA_API_KEY";
const OLLAMA_DEFAULT_API_BASE: &str = "http://localhost:11434";
const PETALS_MISSING_BASE: &str = "the petals arm called no handler without an api_base";

// what get_llm_provider settles before main.py hands the call to an arm
struct PreResolved {
    api_base: Option<String>,
    api_key: Option<SecretString>,
}

pub(super) fn run(case: &Case) -> HelperResult {
    let handler = Recorder::for_case(case);
    let outcome = dispatched(case, &handler)?;
    let call = handler.first()?;
    let (url, headers) = chat_request(case.provider, &call)?;
    let kept = kept_authorization(&header_map(&headers).or_value_error()?);
    Ok(json!({"request": {"url": url, "headers": kept}, "error": error_or_null(&outcome)}))
}

fn dispatched(
    case: &Case,
    handler: &Recorder,
) -> Result<Result<String, DispatchError>, Box<HelperError>> {
    let env = environment(case);
    let settings = section_settings(case, &env);
    let empty = Object::new();
    let optional_params = case.optional_params()?;
    let resolved = pre_resolved(case)?;
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_value_error()?;
    let (mut state, prepost) = logging(&redactor)?;
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
    let mut log = LogContext {
        state: &mut state,
        env: &prepost,
    };
    dispatched_arm(case.provider, &context, handler, &mut log)
}

// lemonade has its own entry; every other chat arm is one row of the dispatch table
fn dispatched_arm(
    provider: Provider,
    context: &DispatchContext<'_>,
    handler: &Recorder,
    log: &mut LogContext<'_, '_>,
) -> Result<Result<String, DispatchError>, Box<HelperError>> {
    if matches!(provider, Provider::Lemonade) {
        return Ok(complete_lemonade(context, handler));
    }
    let tag = provider
        .chat_tag()
        .ok_or_else(|| input_failure(format!("{} has no chat arm", provider.slug())))?;
    dispatch(tag, context, handler, log)
        .ok_or_else(|| input_failure(String::from("the chat table has no arm")))
}

fn pre_resolved(case: &Case) -> Result<PreResolved, Box<HelperError>> {
    let env = environment(case);
    let base = substituted_base(case);
    let key = case.api_key.as_deref().map(SecretString::from);
    let ollama_key = env.get(OLLAMA_API_KEY).cloned().map(SecretString::from);
    match case.provider {
        Provider::HostedVllm => {
            let provider_env = ProviderEnvironment {
                api_base: env.get(HOSTED_VLLM_API_BASE).cloned(),
                api_key: env
                    .get(HOSTED_VLLM_API_KEY)
                    .cloned()
                    .map(SecretString::from),
            };
            let info = provider_info(base.as_deref(), key.as_ref(), &provider_env);
            Ok(PreResolved {
                api_base: info.api_base,
                api_key: Some(info.api_key),
            })
        }
        Provider::Ollama => Ok(PreResolved {
            api_base: Some(ollama_base(base.as_deref(), &env)),
            api_key: first_secret([key.as_ref(), ollama_key.as_ref()]),
        }),
        Provider::Lemonade => {
            let (api_base, api_key) = lemonade_resolved(case)?;
            Ok(PreResolved {
                api_base,
                api_key: api_key.map(SecretString::from),
            })
        }
        // ollama_chat and petals take the passed values alone; nothing else is settled for them
        Provider::OllamaChat | Provider::Petals => Ok(PreResolved {
            api_base: base.filter(|value| !value.is_empty()),
            api_key: first_secret([key.as_ref()]),
        }),
        Provider::Triton | Provider::Oobabooga | Provider::Xinference => Err(input_failure(
            format!("{} has no chat request to record", case.provider.slug()),
        )),
    }
}

fn substituted_base(case: &Case) -> Option<String> {
    case.api_base
        .as_deref()
        .map(|value| value.replace("{{upstream}}", ORACLE_UPSTREAM))
}

// the ollama base chain LiteLLM walks before the arm sends a request, where an empty value falls through
fn ollama_base(passed_base: Option<&str>, env: &BTreeMap<String, String>) -> String {
    [passed_base, env.get(OLLAMA_API_BASE).map(String::as_str)]
        .into_iter()
        .flatten()
        .find(|value| !value.is_empty())
        .map_or_else(|| String::from(OLLAMA_DEFAULT_API_BASE), str::to_owned)
}

fn first_secret<const N: usize>(candidates: [Option<&SecretString>; N]) -> Option<SecretString> {
    candidates
        .into_iter()
        .flatten()
        .find(|key| !key.expose_secret().is_empty())
        .cloned()
}

fn lemonade_resolved(case: &Case) -> Result<(Option<String>, Option<String>), Box<HelperError>> {
    let env = environment(case);
    let credentials = GlobalCredentials {
        api_key: None,
        api_base: None,
        xai_key: None,
        inception_key: None,
        lemonade_key: case.globals.lemonade_key.as_deref().map(SecretString::from),
    };
    let key = case.api_key.as_deref().filter(|value| !value.is_empty());
    Ok(resolve_special(
        substituted_base(case).as_deref(),
        key,
        lemonade_defaults()?,
        &credentials,
        &env,
    ))
}

fn lemonade_defaults() -> Result<&'static SpecialDefaults, Box<HelperError>> {
    SPECIAL_TABLE
        .iter()
        .find_map(|(id, defaults)| (*id == ProviderId::Lemonade).then_some(defaults))
        .ok_or_else(|| input_failure(String::from("the lemonade provider defaults are missing")))
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
        Provider::Petals => call.api_base.clone().map_or_else(
            || Err(input_failure(String::from(PETALS_MISSING_BASE))),
            |api_base| Ok((api_base, call.headers.clone().unwrap_or_default())),
        ),
        Provider::Triton | Provider::Oobabooga | Provider::Xinference => Err(input_failure(
            format!("{} has no chat request to record", provider.slug()),
        )),
    }
}
