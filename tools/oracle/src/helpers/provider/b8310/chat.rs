use std::collections::BTreeMap;
use std::future::{Future, ready};

use betterlite_openai::fireworks_ai_chat_params::FireworksCapabilities;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_prompts::arguments::json::Object;
use betterlite_provider::hosted_platform_dispatch::chat::dispatch;
use betterlite_provider::hosted_platform_dispatch::context::{
    CallLogging, DispatchContext, DispatchSettings,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use secrecy::SecretString;
use serde_json::json;
use tokio::runtime::Runtime;

use super::fixtures::{Recorder, environment, logging, section_settings};
use super::input::{Case, Provider, as_object};
use super::input_failure;
use super::wire::{error_or_null, params_view, request_view};

const GROQ_API_BASE: &str = "GROQ_API_BASE";
const GROQ_API_KEY: &str = "GROQ_API_KEY";
const GROQ_DEFAULT_API_BASE: &str = "https://api.groq.com/openai/v1";
const CHAT_ENDPOINT: &str = "/chat/completions";

// what get_llm_provider settles before main.py hands the call to an arm
struct PreResolved {
    api_base: Option<String>,
    api_key: Option<SecretString>,
}

pub(super) fn run(runtime: &Runtime, case: &Case) -> HelperResult {
    let env = environment(case);
    let settings = section_settings(case, &env);
    let optional_params = as_object(case.optional_params.as_ref())?;
    let groq_config = as_object(case.groq_config.as_ref())?;
    let pre = pre_resolved(case, &env);
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_value_error()?;
    let (mut state, prepost) = logging(&redactor)?;
    let handler = Recorder::for_case(case);
    let empty = Object::new();
    let tag = case.provider.chat_tag();
    let context = DispatchContext {
        acompletion: false,
        api_base: pre.api_base.as_deref(),
        api_key: pre.api_key.as_ref(),
        custom_llm_provider: case.provider.slug(),
        groq_config: &groq_config,
        headers: case.headers.as_ref(),
        litellm_params: &empty,
        messages: &case.messages,
        model: &case.model,
        optional_params: &optional_params,
        provider_config: Some(tag),
        stream: case.stream,
        timeout: None,
        settings: DispatchSettings::from(&settings),
    };
    let mut capabilities = Capabilities {
        reasoning: case.reasoning,
    };
    let mut log = CallLogging {
        state: &mut state,
        env: &prepost,
    };
    let outcome = runtime
        .block_on(dispatch(
            tag,
            &context,
            &mut capabilities,
            &handler,
            &mut log,
        ))
        .ok_or_else(|| input_failure(format!("{} has no chat arm", case.provider.slug())))?;
    let call = handler.first()?;
    let base = call
        .api_base
        .as_deref()
        .ok_or_else(|| input_failure(String::from("the chat arm resolved no api_base")))?;
    let url = match case.provider {
        Provider::Cloudflare => base.to_owned(),
        Provider::FireworksAi | Provider::TogetherAi | Provider::Groq => {
            format!("{base}{CHAT_ENDPOINT}")
        }
    };
    Ok(json!({
        "request": request_view(&url, call.api_key.as_ref(), call.headers.as_ref()),
        "params": params_view(&call.optional_params)?,
        "error": error_or_null(&outcome),
    }))
}

// get_llm_provider fills Groq's base and key before the arm runs, so the arm sees these values; the other providers reach it with the caller's own
fn pre_resolved(case: &Case, env: &BTreeMap<String, String>) -> PreResolved {
    match case.provider {
        Provider::Groq => PreResolved {
            api_base: Some(
                case.api_base
                    .as_deref()
                    .filter(|text| !text.is_empty())
                    .or_else(|| {
                        env.get(GROQ_API_BASE)
                            .map(String::as_str)
                            .filter(|text| !text.is_empty())
                    })
                    .unwrap_or(GROQ_DEFAULT_API_BASE)
                    .to_owned(),
            ),
            api_key: case
                .api_key
                .as_deref()
                .filter(|text| !text.is_empty())
                .or_else(|| {
                    env.get(GROQ_API_KEY)
                        .map(String::as_str)
                        .filter(|text| !text.is_empty())
                })
                .map(SecretString::from),
        },
        Provider::FireworksAi | Provider::TogetherAi | Provider::Cloudflare => PreResolved {
            api_base: case.api_base.clone(),
            api_key: case.api_key.as_deref().map(SecretString::from),
        },
    }
}

// B2221 owns the model lookup; the case states the answer the arm is given
struct Capabilities {
    reasoning: bool,
}

impl FireworksCapabilities for Capabilities {
    fn supports_function_calling(&mut self, _model: &str) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn supports_tool_choice(&mut self, _model: &str) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn supports_reasoning(&mut self, _model: &str) -> impl Future<Output = bool> + Send {
        ready(self.reasoning)
    }
}
