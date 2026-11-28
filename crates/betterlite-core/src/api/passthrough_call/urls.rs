use betterlite_context::resource_config_selection::passthrough::PassthroughConfigTag;
use betterlite_foundry::passthrough::plan::FoundryPassthroughPlan;
use betterlite_openai::gigachat_passthrough::GigachatPassthroughConfig;
use betterlite_openai::nvidia_nim_passthrough::NvidiaNimPassthroughConfig;
use betterlite_openai::passthrough::transformation::BasePassthroughConfig;
use betterlite_openai::passthrough::transformation::types::PassthroughUrlRequest;
use betterlite_openai::watsonx_auth::environment::WatsonxEnvironment;
use betterlite_openai::watsonx_passthrough::WatsonxPassthroughConfig;
use url::Url;

use super::aws::{AwsFlavor, AwsPassthrough};
use super::config::{ConfigError, PassthroughCallConfig, azure_globals, foundry_plan};

// VLLM's ValueError text, braces and all: LiteLLM never formats the placeholder
const VLLM_MISSING_BASE: &str = "VLLM_API_BASE is not set. Please set the environment variable, to use VLLM's pass-through - `{LITELLM_API_BASE}/vllm/{endpoint}`.";

pub(crate) fn complete_url(
    config: &PassthroughCallConfig,
    request: &PassthroughUrlRequest<'_>,
) -> Result<(Url, String), ConfigError> {
    match config.tag {
        PassthroughConfigTag::VLLM => {
            let base = request
                .api_base
                .filter(|base| !base.is_empty())
                .ok_or_else(|| ConfigError::ValueError {
                    message: VLLM_MISSING_BASE.to_owned(),
                })?;
            let url =
                config.format_url(request.target.endpoint, base, request.request_query_params)?;
            Ok((url, base.to_owned()))
        }
        PassthroughConfigTag::Bedrock => {
            AwsPassthrough::complete_url(config.aws.as_ref(), AwsFlavor::Bedrock, request)
        }
        PassthroughConfigTag::BedrockMantle => {
            AwsPassthrough::complete_url(config.aws.as_ref(), AwsFlavor::Mantle, request)
        }
        PassthroughConfigTag::Azure => {
            let azure = azure_globals(config.environment.as_ref())?;
            merged_url(&azure.config, request)
        }
        PassthroughConfigTag::AzureAI => {
            let plan: FoundryPassthroughPlan = foundry_plan(request.litellm_params.clone());
            merged_url(&plan, request)
        }
        PassthroughConfigTag::GigaChat => {
            // get_llm_provider already applied GIGACHAT_API_BASE and GIGACHAT_API_KEY, so the config's env fields stay empty
            let gigachat: GigachatPassthroughConfig = GigachatPassthroughConfig::default();
            merged_url(&gigachat, request)
        }
        PassthroughConfigTag::Watsonx => watsonx_url(config, request),
        PassthroughConfigTag::NvidiaNim => {
            // get_llm_provider already applied NVIDIA_NIM_API_BASE and NVIDIA_NIM_API_KEY, so the config's env fields stay empty
            let nim: NvidiaNimPassthroughConfig = NvidiaNimPassthroughConfig::default();
            merged_url(&nim, request)
        }
    }
}

// the merged per-provider configs own their URL rules, so this call only hands them the resolved request
fn merged_url<Config>(
    config: &Config,
    request: &PassthroughUrlRequest<'_>,
) -> Result<(Url, String), ConfigError>
where
    Config: BasePassthroughConfig,
    Config::Error: ToString,
{
    BasePassthroughConfig::get_complete_url(config, request).map_err(|error| ConfigError::Message {
        status: 500,
        message: error.to_string(),
    })
}

fn watsonx_url(
    config: &PassthroughCallConfig,
    request: &PassthroughUrlRequest<'_>,
) -> Result<(Url, String), ConfigError> {
    let environment = WatsonxEnvironment::from_lookup(config.environment.as_ref());
    let watsonx = WatsonxPassthroughConfig {
        environment: &environment,
    };
    BasePassthroughConfig::get_complete_url(&watsonx, request).map_err(|error| {
        error.status().map_or_else(
            || ConfigError::ValueError {
                message: error.to_string(),
            },
            |status| ConfigError::Message {
                status,
                message: error.to_string(),
            },
        )
    })
}
