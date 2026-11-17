use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::compatible_embedding_settings;
use betterlite_openai::compatible_wire::{EndpointType, header_map, validate_environment};
use betterlite_openai::hosted_vllm::chat::provider_info;
use betterlite_openai::hosted_vllm::embedding as hosted_vllm_embedding;
use betterlite_openai::local_servers::ProviderEnvironment;
use betterlite_openai::local_servers::oobabooga;
use betterlite_openai::openai_base_config::Headers;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_prompts::arguments::json::Object;
use betterlite_provider::ollama_generate::embed::embed_url;
use betterlite_provider::self_hosted_dispatch::call::{
    EmbeddingCall, EmbeddingProvider, EmbeddingRequest,
};
use betterlite_provider::self_hosted_dispatch::context::{DispatchSettings, EmbeddingContext};
use betterlite_provider::self_hosted_dispatch::embedding::dispatch;
use secrecy::SecretString;
use serde_json::json;

use super::fixtures::{environment, section_settings};
use super::input::{Case, Provider};
use super::input_failure;
use super::wire::{kept_authorization, request_view};

const HOSTED_VLLM_API_BASE: &str = "HOSTED_VLLM_API_BASE";
const HOSTED_VLLM_API_KEY: &str = "HOSTED_VLLM_API_KEY";
const OLLAMA_API_BASE: &str = "OLLAMA_API_BASE";
const OLLAMA_API_KEY: &str = "OLLAMA_API_KEY";
const OLLAMA_DEFAULT_API_BASE: &str = "http://localhost:11434";

pub(super) fn run(case: &Case) -> HelperResult {
    let env = environment(case);
    let settings = section_settings(case, &env);
    let (api_base, api_key) = pre_resolved(case.provider, case, &env)?;
    let call = EmbeddingCall {
        model: case.model.clone(),
        input: case.embedding_input()?,
        optional_params: case.optional_params()?,
        litellm_params: Object::new(),
        api_key,
        api_base,
        headers: case.headers.clone(),
        timeout: Duration::from_secs(600),
        aembedding: false,
    };
    let compatible = compatible_embedding_settings::Settings::default();
    let globals = compatible_embedding_settings::Globals::default();
    let context = EmbeddingContext {
        settings: DispatchSettings::from(&settings),
        environment: &env,
        compatible: &compatible,
        globals: &globals,
    };
    let provider = case
        .provider
        .embedding_id()
        .ok_or_else(|| input_failure(String::from("this provider has no embedding arm")))?;
    let request = dispatch(provider, &call, &context)
        .ok_or_else(|| input_failure(String::from("the embedding table has no arm")))?
        .map_err(|error| input_failure(format!("the embedding arm failed: {error}")))?;
    let (url, headers) = embedding_request(&call, &env, request)?;
    let view = request_view(
        &url,
        kept_authorization(&header_map(&headers).or_value_error()?),
    );
    Ok(json!({"request": view}))
}

// what get_llm_provider settles before the embedding table picks the arm
fn pre_resolved(
    provider: Provider,
    case: &Case,
    env: &BTreeMap<String, String>,
) -> Result<(Option<String>, Option<SecretString>), Box<HelperError>> {
    let passed_base = case.api_base.as_deref();
    let passed_key = case.api_key.as_deref().map(SecretString::from);
    match provider {
        Provider::HostedVllm => {
            let environment = ProviderEnvironment {
                api_base: env.get(HOSTED_VLLM_API_BASE).cloned(),
                api_key: env.get(HOSTED_VLLM_API_KEY).map(SecretString::from),
            };
            let info = provider_info(passed_base, passed_key.as_ref(), &environment);
            Ok((info.api_base, Some(info.api_key)))
        }
        Provider::Ollama => {
            let api_base = passed_base
                .filter(|base| !base.is_empty())
                .map(str::to_owned)
                .or_else(|| {
                    env.get(OLLAMA_API_BASE)
                        .filter(|base| !base.is_empty())
                        .cloned()
                })
                .unwrap_or_else(|| String::from(OLLAMA_DEFAULT_API_BASE));
            let api_key = passed_key.or_else(|| {
                env.get(OLLAMA_API_KEY)
                    .filter(|key| !key.is_empty())
                    .map(SecretString::from)
            });
            Ok((Some(api_base), api_key))
        }
        Provider::Triton | Provider::Oobabooga | Provider::Xinference => {
            Ok((passed_base.map(str::to_owned), passed_key))
        }
        Provider::OllamaChat | Provider::Lemonade | Provider::Petals => {
            Err(input_failure(format!("{provider:?} has no embedding arm")))
        }
    }
}

fn embedding_request(
    call: &EmbeddingCall,
    env: &BTreeMap<String, String>,
    request: EmbeddingRequest,
) -> Result<(String, Headers), Box<HelperError>> {
    match request {
        EmbeddingRequest::Compatible {
            provider,
            api_base,
            api_key,
            headers,
            ..
        } => match provider {
            EmbeddingProvider::HostedVllm => Ok((
                hosted_vllm_embedding::get_complete_url(api_base.as_deref(), env)
                    .or_value_error()?,
                hosted_vllm_embedding::validate_environment(
                    &headers.unwrap_or_default(),
                    api_key.as_ref(),
                    env,
                ),
            )),
            // TritonEmbeddingConfig leaves the caller's base as the whole URL and sets no Authorization
            EmbeddingProvider::Triton => {
                let base = api_base.ok_or_else(|| {
                    input_failure(String::from("triton needs an api_base for its embeddings"))
                })?;
                Ok((base, Headers::new()))
            }
            EmbeddingProvider::Oobabooga => Ok((
                oobabooga::embeddings_url(&call.model, api_base.as_deref()).or_value_error()?,
                oobabooga::validate_environment(api_key.as_ref()),
            )),
            EmbeddingProvider::Xinference => Ok(validate_environment(
                api_key.as_ref(),
                api_base.as_deref(),
                EndpointType::Embeddings,
                headers,
                false,
            )
            .or_value_error()?),
            EmbeddingProvider::Ollama
            | EmbeddingProvider::Llamafile
            | EmbeddingProvider::LmStudio => Err(input_failure(String::from(
                "this embedding arm does not send a plain compatible request",
            ))),
        },
        EmbeddingRequest::Ollama { api_base, .. } => Ok((embed_url(&api_base), Headers::new())),
        EmbeddingRequest::Shared { .. } => Err(input_failure(String::from(
            "llamafile and lm_studio embeddings belong to B1440's shared call",
        ))),
    }
}
