use std::collections::BTreeMap;
use std::future::{Future, ready};

use betterlite_anthropic::anthropic_capability_catalog::OptionalParams;
use betterlite_anthropic::anthropic_capability_catalog::thinking::ThinkingError;
use betterlite_anthropic::capabilities::reasoning::ReasoningEffortError;
use betterlite_config::loader::ConfigLoader;
use betterlite_core::api::anthropic_beta_filter::Headers;
use betterlite_core::api::messages_native_transform::capabilities::{
    LegacyShapes, Thinking, ThinkingCapabilities,
};
use betterlite_core::json_messages_policy::{
    JsonMessagesPolicy, MessagesCredentials, generic_complete_url, generic_transform_request,
    generic_validate_environment,
};
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::json_provider_registry::{
    JSONProviderRegistry, ProviderConfig, ProviderDescriptor,
};
use betterlite_openai::json_provider_policy::{
    OpenAiLikeResponsesPolicy, ResponsesPolicy, ResponsesPolicyCache,
};
use betterlite_openai::openai_base_config::Headers as OpenAiHeaders;
use betterlite_openai::response_input::ResponsesInput;
use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    GenericMessagesUrl,
    JsonMessagesUrl,
    GenericMessagesHeaders,
    JsonMessagesHeaders,
    GenericMessagesTransform,
    JsonMessagesTransform,
    ResponsesUrl,
    ResponsesHeaders,
    ResponsesTransform,
    ResponsesCache,
    GenericResponsesUrl,
    GenericResponsesHeaders,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CaseInput {
    calls: Vec<Call>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    kind: Kind,
    model: String,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    global_api_key: Option<String>,
    #[serde(default)]
    descriptor: Option<ProviderConfig>,
    #[serde(default)]
    headers: IndexMap<String, String>,
    #[serde(default)]
    optional_params: Object,
    #[serde(default)]
    messages: Vec<Json>,
    #[serde(default)]
    input: String,
    #[serde(default)]
    params: Object,
    #[serde(default)]
    first: Option<ProviderConfig>,
    #[serde(default)]
    second: Option<ProviderConfig>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = CaseInput;

    fn run(input: &Self::Input) -> HelperResult {
        let registry = JSONProviderRegistry::load();
        let mut results: Vec<Value> = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            results.push(run_call(call, &registry)?);
        }
        Ok(Value::Array(results))
    }
}

fn run_call(call: &Call, registry: &JSONProviderRegistry) -> Result<Value, Box<HelperError>> {
    let (slug, model) = split_model(&call.model)?;
    let provider = provider_id(&slug)?;
    match call.kind {
        Kind::GenericMessagesUrl => {
            let url = generic_complete_url(call.api_base.as_deref())
                .map_err(|error| helper_error("ValueError", error.to_string()))?;
            Ok(Value::String(url))
        }
        Kind::JsonMessagesUrl => {
            let policy = messages_policy(call, registry, provider)?;
            let (explicit, global) = keys(call);
            let credentials = MessagesCredentials {
                api_base: call.api_base.as_deref(),
                api_key: explicit.as_ref(),
                global_api_key: global.as_ref(),
                lookup: &call.environment,
            };
            let url = policy
                .complete_url(&credentials)
                .map_err(|error| helper_error("ValueError", error.to_string()))?;
            Ok(Value::String(url))
        }
        Kind::GenericMessagesHeaders => {
            let (explicit, _) = keys(call);
            let headers = generic_validate_environment(
                headers(call),
                explicit.as_ref(),
                &call.optional_params,
                &call.messages,
            )
            .map_err(|error| helper_error("ValueError", error.to_string()))?;
            to_value(headers)
        }
        Kind::JsonMessagesHeaders => {
            let policy = messages_policy(call, registry, provider)?;
            let (explicit, global) = keys(call);
            let credentials = MessagesCredentials {
                api_base: call.api_base.as_deref(),
                api_key: explicit.as_ref(),
                global_api_key: global.as_ref(),
                lookup: &call.environment,
            };
            let headers = policy
                .validate_environment(
                    headers(call),
                    &credentials,
                    &call.optional_params,
                    &call.messages,
                )
                .map_err(|error| helper_error("ValueError", error.to_string()))?;
            to_value(headers)
        }
        Kind::GenericMessagesTransform => {
            let runtime = runtime()?;
            let body = runtime
                .block_on(generic_transform_request(
                    &mut NoCapabilities,
                    &model,
                    call.messages.clone(),
                    call.optional_params.clone(),
                ))
                .map_err(|error| helper_error("ValueError", error.to_string()))?;
            to_value(body)
        }
        Kind::JsonMessagesTransform => {
            let policy = messages_policy(call, registry, provider)?;
            let runtime = runtime()?;
            let body = runtime
                .block_on(policy.transform_request(
                    &mut NoCapabilities,
                    &model,
                    call.messages.clone(),
                    call.optional_params.clone(),
                ))
                .map_err(|error| helper_error("ValueError", error.to_string()))?;
            to_value(body)
        }
        Kind::ResponsesUrl => {
            let policy = responses_policy(call, registry, provider)?;
            Ok(Value::String(
                policy.complete_url(call.api_base.as_deref(), &call.environment),
            ))
        }
        Kind::ResponsesHeaders => {
            let policy = responses_policy(call, registry, provider)?;
            let (explicit, _) = keys(call);
            let headers = policy.validate_environment(
                responses_headers(call),
                explicit.as_ref(),
                &call.environment,
            );
            to_value(headers)
        }
        Kind::ResponsesTransform => {
            let policy = responses_policy(call, registry, provider)?;
            let body = policy
                .transform_request(
                    &model,
                    &ResponsesInput::Text(call.input.clone()),
                    &call.params,
                    None,
                )
                .map_err(|error| helper_error("ValueError", error.to_string()))?;
            to_value(body)
        }
        Kind::ResponsesCache => cache_value(call, &slug, provider),
        Kind::GenericResponsesUrl => {
            let settings = openai_like_settings(call)?;
            let url = OpenAiLikeResponsesPolicy::new(settings)
                .complete_url(call.api_base.as_deref())
                .map_err(|error| helper_error("ValueError", error.to_string()))?;
            Ok(Value::String(url))
        }
        Kind::GenericResponsesHeaders => {
            let settings = openai_like_settings(call)?;
            let (explicit, _) = keys(call);
            let headers = OpenAiLikeResponsesPolicy::new(settings)
                .validate_environment(responses_headers(call), explicit.as_ref());
            to_value(headers)
        }
    }
}

fn split_model(model: &str) -> Result<(String, String), Box<HelperError>> {
    let Some((slug, rest)) = model.split_once('/') else {
        return Err(helper_error(
            "ValueError",
            format!("the case model {model} carries no provider prefix"),
        ));
    };
    Ok((slug.to_owned(), rest.to_owned()))
}

fn provider_id(slug: &str) -> Result<ProviderId, Box<HelperError>> {
    slug.parse()
        .map_err(|error| helper_error("ValueError", format!("{slug}: {error}")))
}

fn keys(call: &Call) -> (Option<SecretString>, Option<SecretString>) {
    (
        call.api_key.clone().map(SecretString::from),
        call.global_api_key.clone().map(SecretString::from),
    )
}

fn headers(call: &Call) -> Headers {
    call.headers.clone()
}

// the Responses policies carry the OpenAI crate's sorted header map
fn responses_headers(call: &Call) -> OpenAiHeaders {
    call.headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

fn messages_policy(
    call: &Call,
    registry: &JSONProviderRegistry,
    provider: ProviderId,
) -> Result<JsonMessagesPolicy, Box<HelperError>> {
    if let Some(config) = &call.descriptor {
        return Ok(JsonMessagesPolicy::new(ProviderDescriptor {
            provider,
            config: config.clone(),
        }));
    }
    JsonMessagesPolicy::for_provider(registry, provider).ok_or_else(|| {
        helper_error(
            "ValueError",
            format!("{provider} is not a JSON provider with a messages descriptor"),
        )
    })
}

fn responses_policy(
    call: &Call,
    registry: &JSONProviderRegistry,
    provider: ProviderId,
) -> Result<ResponsesPolicy, Box<HelperError>> {
    if let Some(config) = &call.descriptor {
        return Ok(ResponsesPolicy::new(ProviderDescriptor {
            provider,
            config: config.clone(),
        }));
    }
    ResponsesPolicy::for_provider(registry, provider)
        .ok_or_else(|| helper_error("ValueError", format!("{provider} is not a JSON provider")))
}

// create_responses_config_class caches by slug for the process, so the first descriptor's class answers both
fn cache_value(call: &Call, slug: &str, provider: ProviderId) -> Result<Value, Box<HelperError>> {
    let first = call.first.clone().ok_or_else(|| {
        helper_error(
            "ValueError",
            format!("the {slug} cache call needs two descriptors"),
        )
    })?;
    let second = call.second.clone().ok_or_else(|| {
        helper_error(
            "ValueError",
            format!("the {slug} cache call needs two descriptors"),
        )
    })?;
    let lookup = BTreeMap::<String, String>::new();
    let mut cache = ResponsesPolicyCache::new();
    let first_url = cache
        .policy_for_descriptor(ProviderDescriptor {
            provider,
            config: first,
        })
        .complete_url(None, &lookup);
    let second_url = cache
        .policy_for_descriptor(ProviderDescriptor {
            provider,
            config: second,
        })
        .complete_url(None, &lookup);
    Ok(json!({
        "first": first_url,
        "second": second_url,
        "cached": cache.len() == 1,
    }))
}

fn openai_like_settings(
    call: &Call,
) -> Result<betterlite_config::sections::openai_like_settings::Settings, Box<HelperError>> {
    ConfigLoader {
        lookup: call.environment.clone(),
    }
    .load("")
    .map(|config| config.sections.openai_like_settings)
    .map_err(|error| helper_error("ValueError", error.to_string()))
}

fn runtime() -> Result<tokio::runtime::Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .build()
        .map_err(|error| helper_error("RuntimeError", error.to_string()))
}

fn to_value<T>(value: T) -> Result<Value, Box<HelperError>>
where
    T: serde::Serialize,
{
    serde_json::to_value(value).map_err(|error| helper_error("ValueError", error.to_string()))
}

fn helper_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

// the cases carry no thinking fields, so the fake never answers a capability question with data
struct NoCapabilities;

impl ThinkingCapabilities for NoCapabilities {
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
