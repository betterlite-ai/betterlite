use betterlite_config::api::rerank_provider_plan::RerankSettings;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_error::dispatch::RawException;
use betterlite_models::resolution::ResolvedModel;
use secrecy::SecretString;

use super::call::GenericParams;
use super::dispatch::{Headers, Params, value_failure};
use super::selection::SelectedConfig;
use watsonx::{WatsonxRequest, watsonx_credentials};

mod watsonx;

pub(super) struct Credentials {
    pub(super) api_key: Option<SecretString>,
    pub(super) api_base: Option<String>,
    pub(super) headers: Headers,
}

pub(super) struct BranchInput<'request> {
    pub(super) provider: &'request str,
    pub(super) model: &'request str,
    pub(super) config: SelectedConfig,
    pub(super) resolved: &'request ResolvedModel,
    pub(super) generic: &'request GenericParams,
    pub(super) generic_params: &'request Params,
    pub(super) settings: &'request RerankSettings,
    pub(super) environment: &'request dyn EnvironmentLookup,
    pub(super) headers: Headers,
}

// Every branch of the dispatcher: the key and base chain it reads and the headers it hands its handler
pub(super) fn select_credentials(
    input: BranchInput<'_>,
    mapped: &mut Params,
) -> Result<Credentials, RawException> {
    let BranchInput {
        provider,
        model,
        config,
        resolved,
        generic,
        generic_params,
        settings,
        environment: lookup,
        mut headers,
    } = input;
    let environment = settings.environment.resolve(lookup);
    if provider == "bedrock" {
        if let Some(extra) = generic
            .extra_headers
            .as_ref()
            .filter(|extra| !extra.is_empty())
        {
            for (name, value) in extra {
                headers.insert(name.clone(), value.clone());
            }
        }
    }
    if provider == "watsonx" {
        return watsonx_credentials(
            WatsonxRequest {
                model,
                provider,
                resolved,
                generic_params,
                environment: lookup,
                headers,
            },
            mapped,
        );
    }
    let (api_key, api_base) = match provider {
        "cohere" | "litellm_proxy" => (
            resolved
                .api_key
                .clone()
                .or(generic.api_key.clone())
                .or(settings.api_key.clone()),
            first_text(&[
                resolved.api_base.as_deref(),
                generic.api_base.as_deref(),
                settings.api_base.as_deref(),
                environment.cohere_api_base.as_deref(),
                Some("https://api.cohere.com"),
            ]),
        ),
        "azure_ai" => (
            resolved.api_key.clone().or(generic.api_key.clone()),
            first_text(&[
                resolved.api_base.as_deref(),
                generic.api_base.as_deref(),
                settings.api_base.as_deref(),
                environment.azure_ai_api_base.as_deref(),
            ]),
        ),
        "infinity" => {
            let Some(api_base) = first_text(&[
                resolved.api_base.as_deref(),
                generic.api_base.as_deref(),
                settings.api_base.as_deref(),
                environment.infinity_api_base.as_deref(),
            ]) else {
                return Err(value_failure(
                    "Invalid api base. api_base=None. Set in call or via `INFINITY_API_BASE` env var.",
                    model,
                    provider,
                ));
            };
            (
                // the branch computes a global key and then never sends it
                resolved.api_key.clone().or(generic.api_key.clone()),
                Some(api_base),
            )
        }
        "together_ai" => {
            let Some(api_key) = resolved
                .api_key
                .clone()
                .or(generic.api_key.clone())
                .or(settings.togetherai_api_key.clone())
                .or(environment.togetherai_api_key.clone())
                .or(settings.api_key.clone())
            else {
                return Err(value_failure(
                    "TogetherAI API key is required, please set 'TOGETHERAI_API_KEY' in your environment",
                    model,
                    provider,
                ));
            };
            (
                Some(api_key),
                first_text(&[
                    resolved.api_base.as_deref(),
                    generic.api_base.as_deref(),
                    settings.api_base.as_deref(),
                    Some("https://api.together.ai/v1"),
                ]),
            )
        }
        "jina_ai" => {
            let Some(api_key) = resolved.api_key.clone() else {
                return Err(value_failure(
                    "Jina AI API key is required, please set 'JINA_AI_API_KEY' in your environment",
                    model,
                    provider,
                ));
            };
            (
                Some(api_key),
                first_text(&[
                    resolved.api_base.as_deref(),
                    generic.api_base.as_deref(),
                    settings.api_base.as_deref(),
                    environment.bedrock_api_base.as_deref(),
                ]),
            )
        }
        "nvidia_nim" => {
            let Some(api_key) = resolved.api_key.clone() else {
                return Err(value_failure(
                    "Nvidia NIM API key is required, please set 'NVIDIA_NIM_API_KEY' in your environment",
                    model,
                    provider,
                ));
            };
            (
                Some(api_key),
                // the rerank branch ignores the resolved and global bases
                first_text(&[
                    generic.api_base.as_deref(),
                    environment.nvidia_nim_api_base.as_deref(),
                    Some("https://ai.api.nvidia.com"),
                ]),
            )
        }
        "bedrock" => (
            resolved.api_key.clone().or(generic.api_key.clone()),
            first_text(&[
                resolved.api_base.as_deref(),
                generic.api_base.as_deref(),
                settings.api_base.as_deref(),
                environment.bedrock_api_base.as_deref(),
            ]),
        ),
        "hosted_vllm" => {
            let Some(api_base) = first_text(&[
                resolved.api_base.as_deref(),
                generic.api_base.as_deref(),
                environment.hosted_vllm_api_base.as_deref(),
            ]) else {
                return Err(value_failure(
                    "api_base must be provided for Hosted VLLM rerank. Set in call or via HOSTED_VLLM_API_BASE env var.",
                    model,
                    provider,
                ));
            };
            (
                resolved
                    .api_key
                    .clone()
                    .or(generic.api_key.clone())
                    .or(environment.hosted_vllm_api_key.clone()),
                Some(api_base),
            )
        }
        "deepinfra" => {
            let Some(api_base) = first_text(&[
                resolved.api_base.as_deref(),
                generic.api_base.as_deref(),
                environment.deepinfra_api_base.as_deref(),
            ]) else {
                return Err(value_failure(
                    "api_base must be provided for Deepinfra rerank. Set in call or via DEEPINFRA_API_BASE env var.",
                    model,
                    provider,
                ));
            };
            (
                resolved
                    .api_key
                    .clone()
                    .or(generic.api_key.clone())
                    .or(environment.deepinfra_api_key.clone()),
                Some(api_base),
            )
        }
        "fireworks_ai" => (
            resolved
                .api_key
                .clone()
                .or(generic.api_key.clone())
                .or(environment.fireworks_api_key.clone())
                .or(environment.fireworks_ai_api_key.clone())
                .or(environment.fireworksai_api_key.clone())
                .or(environment.fireworks_ai_token.clone()),
            first_text(&[
                resolved.api_base.as_deref(),
                generic.api_base.as_deref(),
                environment.fireworks_ai_api_base.as_deref(),
            ]),
        ),
        "voyage" => (
            resolved
                .api_key
                .clone()
                .or(generic.api_key.clone())
                .or(environment.voyage_api_key.clone())
                .or(environment.voyage_ai_api_key.clone()),
            first_text(&[
                resolved.api_base.as_deref(),
                generic.api_base.as_deref(),
                environment.voyage_api_base.as_deref(),
            ]),
        ),
        other => {
            if matches!(config, SelectedConfig::Cohere | SelectedConfig::CohereV2) {
                return Err(value_failure(
                    &format!("Unsupported provider: {other}"),
                    model,
                    other,
                ));
            }
            (
                resolved.api_key.clone().or(generic.api_key.clone()),
                first_text(&[resolved.api_base.as_deref(), generic.api_base.as_deref()]),
            )
        }
    };
    Ok(Credentials {
        api_key,
        api_base,
        headers,
    })
}

fn first_text(values: &[Option<&str>]) -> Option<String> {
    values
        .iter()
        .copied()
        .flatten()
        .find(|value| !value.is_empty())
        .map(str::to_owned)
}
