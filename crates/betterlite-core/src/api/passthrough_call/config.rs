use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_azure::passthrough::config::AzurePassthroughConfig;
use betterlite_azure::passthrough::logging::{NoChatReplyProjection, NoChatStreamAssembly};
use betterlite_config::sections::Section as _;
use betterlite_config::sections::azure_provider_globals;
use betterlite_context::resource_config_selection::passthrough::PassthroughConfigTag;
use betterlite_foundry::passthrough::plan::FoundryPassthroughPlan;
use betterlite_http::interactions::base::{ErrorClasses, ResponseHeaders};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::error::{ProviderError, ProviderErrorParts};
use betterlite_openai::gigachat_passthrough::GigachatPassthroughConfig;
use betterlite_openai::nvidia_nim_passthrough::NvidiaNimPassthroughConfig;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::passthrough::transformation::types::{
    PassthroughError, PassthroughSigning, PassthroughTarget, PassthroughUrlRequest,
};
use betterlite_openai::passthrough::transformation::{
    BasePassthroughConfig, PassthroughStreamCollector,
};
use betterlite_openai::watsonx_auth::environment::WatsonxEnvironment;
use betterlite_openai::watsonx_passthrough::WatsonxPassthroughConfig;
use thiserror::Error;
use url::Url;

use super::auth;
use super::logging::NoSpendCollector;
use super::urls;

// LiteLLM's ValueError and provider exception text, kept apart so _handle_error can read the status
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error(transparent)]
    Passthrough(#[from] PassthroughError),
    #[error("{message}")]
    Message { status: u16, message: String },
    #[error("{message}")]
    ValueError { message: String },
    #[error("{}", .0.message)]
    Provider(Box<ProviderError>),
}

impl From<ProviderError> for ConfigError {
    fn from(error: ProviderError) -> Self {
        Self::Provider(Box::new(error))
    }
}

// The error a step of the call raises before the send; _handle_error reads its status and text
pub(crate) struct FaultDetail {
    pub status: u16,
    pub synthesized: bool,
    pub message: String,
}

pub(crate) enum Fault {
    Failure(Box<FaultDetail>),
    Send(Box<TransportError>),
}

impl From<ConfigError> for Fault {
    fn from(error: ConfigError) -> Self {
        match error {
            ConfigError::Message { status, message } => Self::Failure(Box::new(FaultDetail {
                status,
                synthesized: false,
                message,
            })),
            ConfigError::ValueError { message } => Self::Failure(Box::new(FaultDetail {
                status: 500,
                synthesized: true,
                message,
            })),
            ConfigError::Passthrough(source) => Self::Failure(Box::new(FaultDetail {
                status: 500,
                synthesized: true,
                message: source.to_string(),
            })),
            ConfigError::Provider(provider) => Self::Failure(Box::new(FaultDetail {
                status: provider.status_code,
                synthesized: provider.status_code_is_synthesized,
                message: provider.message,
            })),
        }
    }
}

// The eight configs LiteLLM's pass-through factory returns, chosen by B5360's tag; the Arc keeps the caller's map alive for the relay
pub struct PassthroughCallConfig {
    pub tag: PassthroughConfigTag,
    pub environment: Arc<BTreeMap<String, String>>,
}

// B10044's reader of the AZURE_* variables, shared by the Azure URL arm and its key branch
pub(crate) struct AzureGlobals {
    pub(crate) config: AzurePassthroughConfig,
    pub(crate) environment: azure_provider_globals::Environment,
}

pub(crate) fn azure_globals(
    environment: &BTreeMap<String, String>,
) -> Result<AzureGlobals, ConfigError> {
    let settings = azure_provider_globals::Raw::default()
        .convert(environment)
        .map_err(|error| ConfigError::ValueError {
            message: error.to_string(),
        })?;
    let config = AzurePassthroughConfig {
        api_base_env: settings.environment.api_base.clone(),
        api_key_env: settings.environment.api_key.clone(),
        model_call_details: None,
        chat_reply: NoChatReplyProjection,
        chat_stream: NoChatStreamAssembly,
    };
    Ok(AzureGlobals {
        config,
        environment: settings.environment,
    })
}

// the Foundry plan the azure_ai tag reads; get_llm_provider already applied AZURE_AI_API_BASE and AZURE_AI_API_KEY, so its env fields stay empty
pub(crate) fn foundry_plan(litellm_params: Object) -> FoundryPassthroughPlan {
    FoundryPassthroughPlan {
        global_api_base: None,
        global_api_key: None,
        azure: AzurePassthroughConfig::default(),
        ocr: None,
        litellm_params,
    }
}

impl BasePassthroughConfig for PassthroughCallConfig {
    type Error = ConfigError;
    type Logged = ();

    // is_streaming_request reads the request data alone, so its provider arms keep their default configs
    fn is_streaming_request(&self, endpoint: &str, request_data: &Object) -> bool {
        match self.tag {
            PassthroughConfigTag::VLLM => request_data.contains_key("stream"),
            PassthroughConfigTag::Bedrock | PassthroughConfigTag::BedrockMantle => {
                endpoint.contains("stream")
            }
            PassthroughConfigTag::Azure => {
                let config: AzurePassthroughConfig = AzurePassthroughConfig::default();
                config.is_streaming_request(endpoint, request_data)
            }
            PassthroughConfigTag::AzureAI => {
                let plan: FoundryPassthroughPlan = foundry_plan(Object::new());
                plan.is_streaming_request(endpoint, request_data)
            }
            PassthroughConfigTag::GigaChat => {
                let config: GigachatPassthroughConfig = GigachatPassthroughConfig::default();
                config.is_streaming_request(endpoint, request_data)
            }
            PassthroughConfigTag::Watsonx => {
                let environment = WatsonxEnvironment::default();
                let config = WatsonxPassthroughConfig {
                    environment: &environment,
                };
                config.is_streaming_request(endpoint, request_data)
            }
            PassthroughConfigTag::NvidiaNim => {
                let config: NvidiaNimPassthroughConfig = NvidiaNimPassthroughConfig::default();
                config.is_streaming_request(endpoint, request_data)
            }
        }
    }

    fn get_complete_url(
        &self,
        request: &PassthroughUrlRequest<'_>,
    ) -> Result<(Url, String), Self::Error> {
        urls::complete_url(self, request)
    }

    fn sign_request(
        &self,
        headers: Headers,
        context: &PassthroughSigning<'_>,
    ) -> Result<(Headers, Option<Vec<u8>>), Self::Error> {
        auth::sign_request(self.tag, headers, context)
    }

    fn create_stream_collector(
        &self,
        _target: &PassthroughTarget<'_>,
    ) -> Box<dyn PassthroughStreamCollector<Logged = Self::Logged> + Send + '_>
    where
        Self: Sized + Sync,
    {
        Box::new(NoSpendCollector)
    }
}

// _handle_error maps a status failure to the provider's own error class through this trait
impl ErrorClasses for PassthroughCallConfig {
    type Error = ConfigError;

    fn get_error_class(
        &self,
        message: &str,
        status_code: u16,
        headers: &ResponseHeaders,
    ) -> Self::Error {
        let parts = ProviderErrorParts {
            status_code,
            message: message.to_owned(),
            headers: Some(
                headers
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone()))
                    .collect(),
            ),
            request: None,
            response: None,
            body: None,
            status_code_is_synthesized: false,
        };
        ConfigError::Provider(Box::new(ProviderError::from(parts)))
    }
}
