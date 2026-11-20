use betterlite_config::api::rerank_provider_plan::RerankSettings;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_error::dispatch::RawException;
use betterlite_models::decisions::Json;
use betterlite_models::resolution::ResolvedModel;
use betterlite_openai::watsonx_auth::credentials::get_watsonx_credentials;
use betterlite_openai::watsonx_auth::environment::WatsonxEnvironment;
use betterlite_openai::watsonx_auth::error::WatsonxError;
use betterlite_openai::watsonx_auth::types::WatsonxParams;
use secrecy::{ExposeSecret as _, SecretString};

use super::call::GenericParams;
use super::dispatch::{Headers, Params, provider_failure, value_failure};
use super::selection::SelectedConfig;

// The credentials one branch hands its handler: the key, the base and the headers that reach the request
pub(super) struct Credentials {
    pub(super) api_key: Option<SecretString>,
    pub(super) api_base: Option<String>,
    pub(super) headers: Headers,
}

// Everything the branch table reads; the mapped options come along for Watsonx's token slot
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

pub(super) fn select_credentials(
    input: BranchInput<'_>,
    mapped: &mut Params,
) -> Result<Credentials, RawException> {
    let resolved = input.resolved;
    let generic = input.generic;
    let settings = input.settings;
    let environment = &settings.environment;
    match input.provider {
        "cohere" | "litellm_proxy" => Ok(Credentials {
            api_key: first_secret(&[
                resolved.api_key.clone(),
                generic.api_key.clone(),
                settings.api_key.clone(),
            ]),
            // the literal fallback makes the branch's own missing-base guard unreachable
            api_base: first_text(&[
                resolved.api_base.as_deref(),
                generic.api_base.as_deref(),
                settings.api_base.as_deref(),
                environment.cohere_api_base.as_deref(),
                Some("https://api.cohere.com"),
            ]),
            headers: input.headers,
        }),
        "azure_ai" => Ok(Credentials {
            api_key: first_secret(&[resolved.api_key.clone(), generic.api_key.clone()]),
            api_base: first_text(&[
                resolved.api_base.as_deref(),
                generic.api_base.as_deref(),
                settings.api_base.as_deref(),
                environment.azure_ai_api_base.as_deref(),
            ]),
            headers: input.headers,
        }),
        "infinity" => {
            let Some(api_base) = first_text(&[
                resolved.api_base.as_deref(),
                generic.api_base.as_deref(),
                settings.api_base.as_deref(),
                environment.infinity_api_base.as_deref(),
            ]) else {
                return Err(value_failure(
                    "Invalid api base. api_base=None. Set in call or via `INFINITY_API_BASE` env var.",
                    input.model,
                    input.provider,
                ));
            };
            Ok(Credentials {
                // the branch computes a global key and then never sends it
                api_key: first_secret(&[resolved.api_key.clone(), generic.api_key.clone()]),
                api_base: Some(api_base),
                headers: input.headers,
            })
        }
        "together_ai" => {
            let Some(api_key) = first_secret(&[
                resolved.api_key.clone(),
                generic.api_key.clone(),
                settings.togetherai_api_key.clone(),
                environment.togetherai_api_key.clone(),
                settings.api_key.clone(),
            ]) else {
                return Err(value_failure(
                    "TogetherAI API key is required, please set 'TOGETHERAI_API_KEY' in your environment",
                    input.model,
                    input.provider,
                ));
            };
            Ok(Credentials {
                api_key: Some(api_key),
                api_base: first_text(&[
                    resolved.api_base.as_deref(),
                    generic.api_base.as_deref(),
                    settings.api_base.as_deref(),
                    Some("https://api.together.ai/v1"),
                ]),
                headers: input.headers,
            })
        }
        "jina_ai" => {
            let Some(api_key) = resolved.api_key.clone() else {
                return Err(value_failure(
                    "Jina AI API key is required, please set 'JINA_AI_API_KEY' in your environment",
                    input.model,
                    input.provider,
                ));
            };
            Ok(Credentials {
                api_key: Some(api_key),
                api_base: first_text(&[
                    resolved.api_base.as_deref(),
                    generic.api_base.as_deref(),
                    settings.api_base.as_deref(),
                    environment.bedrock_api_base.as_deref(),
                ]),
                headers: input.headers,
            })
        }
        "nvidia_nim" => {
            let Some(api_key) = resolved.api_key.clone() else {
                return Err(value_failure(
                    "Nvidia NIM API key is required, please set 'NVIDIA_NIM_API_KEY' in your environment",
                    input.model,
                    input.provider,
                ));
            };
            Ok(Credentials {
                api_key: Some(api_key),
                // the rerank branch ignores the resolved and global bases
                api_base: first_text(&[
                    generic.api_base.as_deref(),
                    environment.nvidia_nim_api_base.as_deref(),
                    Some("https://ai.api.nvidia.com"),
                ]),
                headers: input.headers,
            })
        }
        "bedrock" => {
            let mut headers = input.headers;
            if let Some(extra) = generic
                .extra_headers
                .as_ref()
                .filter(|extra| !extra.is_empty())
            {
                for (name, value) in extra {
                    headers.insert(name.clone(), value.clone());
                }
            }
            Ok(Credentials {
                api_key: first_secret(&[resolved.api_key.clone(), generic.api_key.clone()]),
                api_base: first_text(&[
                    resolved.api_base.as_deref(),
                    generic.api_base.as_deref(),
                    settings.api_base.as_deref(),
                    environment.bedrock_api_base.as_deref(),
                ]),
                headers,
            })
        }
        "hosted_vllm" => {
            let Some(api_base) = first_text(&[
                resolved.api_base.as_deref(),
                generic.api_base.as_deref(),
                environment.hosted_vllm_api_base.as_deref(),
            ]) else {
                return Err(value_failure(
                    "api_base must be provided for Hosted VLLM rerank. Set in call or via HOSTED_VLLM_API_BASE env var.",
                    input.model,
                    input.provider,
                ));
            };
            Ok(Credentials {
                api_key: first_secret(&[
                    resolved.api_key.clone(),
                    generic.api_key.clone(),
                    environment.hosted_vllm_api_key.clone(),
                ]),
                api_base: Some(api_base),
                headers: input.headers,
            })
        }
        "deepinfra" => {
            let Some(api_base) = first_text(&[
                resolved.api_base.as_deref(),
                generic.api_base.as_deref(),
                environment.deepinfra_api_base.as_deref(),
            ]) else {
                return Err(value_failure(
                    "api_base must be provided for Deepinfra rerank. Set in call or via DEEPINFRA_API_BASE env var.",
                    input.model,
                    input.provider,
                ));
            };
            Ok(Credentials {
                api_key: first_secret(&[
                    resolved.api_key.clone(),
                    generic.api_key.clone(),
                    environment.deepinfra_api_key.clone(),
                ]),
                api_base: Some(api_base),
                headers: input.headers,
            })
        }
        "fireworks_ai" => Ok(Credentials {
            api_key: first_secret(&[
                resolved.api_key.clone(),
                generic.api_key.clone(),
                environment.fireworks_api_key.clone(),
                environment.fireworks_ai_api_key.clone(),
                environment.fireworksai_api_key.clone(),
                environment.fireworks_ai_token.clone(),
            ]),
            api_base: first_text(&[
                resolved.api_base.as_deref(),
                generic.api_base.as_deref(),
                environment.fireworks_ai_api_base.as_deref(),
            ]),
            headers: input.headers,
        }),
        "voyage" => Ok(Credentials {
            api_key: first_secret(&[
                resolved.api_key.clone(),
                generic.api_key.clone(),
                environment.voyage_api_key.clone(),
                environment.voyage_ai_api_key.clone(),
            ]),
            api_base: first_text(&[
                resolved.api_base.as_deref(),
                generic.api_base.as_deref(),
                environment.voyage_api_base.as_deref(),
            ]),
            headers: input.headers,
        }),
        "watsonx" => watsonx_credentials(&input, mapped),
        other => {
            if input.config.is_cohere_class() {
                return Err(value_failure(
                    &format!("Unsupported provider: {other}"),
                    input.model,
                    other,
                ));
            }
            Ok(Credentials {
                api_key: first_secret(&[resolved.api_key.clone(), generic.api_key.clone()]),
                api_base: first_text(&[resolved.api_base.as_deref(), generic.api_base.as_deref()]),
                headers: input.headers,
            })
        }
    }
}

// get_watsonx_credentials reads the whole generic map plus the resolved key and base, and keeps a token that is not None
fn watsonx_credentials(
    input: &BranchInput<'_>,
    mapped: &mut Params,
) -> Result<Credentials, RawException> {
    let mut params = WatsonxParams::new();
    for (name, value) in input.generic_params {
        let converted = serde_json::to_value(value).ok();
        if let Some(converted) = converted {
            params.insert(name.clone(), converted);
        }
    }
    let environment = watsonx_environment(input.environment);
    match get_watsonx_credentials(
        &mut params,
        input.resolved.api_key.as_ref(),
        input.resolved.api_base.as_deref(),
        &environment,
    ) {
        Ok(credentials) => {
            if let Some(token) = credentials.token {
                mapped.insert(
                    "token".to_owned(),
                    Json::Text(token.expose_secret().to_owned()),
                );
            }
            Ok(Credentials {
                api_key: Some(credentials.api_key),
                api_base: Some(credentials.api_base),
                headers: input.headers.clone(),
            })
        }
        Err(error) => Err(watsonx_failure(&error, input.model, input.provider)),
    }
}

fn watsonx_failure(error: &WatsonxError, model: &str, provider: &str) -> RawException {
    match error {
        WatsonxError::Api {
            status_code,
            message,
            ..
        } => provider_failure(*status_code, message, model, provider, "WatsonxError"),
        WatsonxError::IamStatus { status, .. } => {
            provider_failure(*status, &error.to_string(), model, provider, "WatsonxError")
        }
        WatsonxError::IamApiKeyRequired
        | WatsonxError::InvalidField { .. }
        | WatsonxError::InvalidHeader { .. }
        | WatsonxError::IamUrl(_)
        | WatsonxError::IamTransport(_)
        | WatsonxError::IamReply(_)
        | WatsonxError::IamReplyField { .. }
        | WatsonxError::Cache(_) => value_failure(&error.to_string(), model, provider),
    }
}

fn watsonx_environment(lookup: &dyn EnvironmentLookup) -> WatsonxEnvironment {
    let text = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
    let secret = |name: &str| text(name).map(SecretString::from);
    WatsonxEnvironment {
        watsonx_iam_url: text("WATSONX_IAM_URL"),
        wx_api_key: secret("WX_API_KEY"),
        watsonx_api_key: secret("WATSONX_API_KEY"),
        watsonx_apikey: secret("WATSONX_APIKEY"),
        watsonx_zenapikey: secret("WATSONX_ZENAPIKEY"),
        watsonx_token: secret("WATSONX_TOKEN"),
        watsonx_api_base: text("WATSONX_API_BASE"),
        watsonx_url: text("WATSONX_URL"),
        wx_url: text("WX_URL"),
        wml_url: text("WML_URL"),
        watsonx_project_id: text("WATSONX_PROJECT_ID"),
        wx_project_id: text("WX_PROJECT_ID"),
        project_id: text("PROJECT_ID"),
        watsonx_region: text("WATSONX_REGION"),
        wx_region: text("WX_REGION"),
        region: text("REGION"),
        watsonx_deployment_space_id: text("WATSONX_DEPLOYMENT_SPACE_ID"),
        watsonx_space_id: text("WATSONX_SPACE_ID"),
        wx_space_id: text("WX_SPACE_ID"),
        space_id: text("SPACE_ID"),
    }
}

fn first_secret(values: &[Option<SecretString>]) -> Option<SecretString> {
    values.iter().find_map(|value| value.clone())
}

fn first_text(values: &[Option<&str>]) -> Option<String> {
    values
        .iter()
        .copied()
        .flatten()
        .find(|value| !value.is_empty())
        .map(str::to_owned)
}
