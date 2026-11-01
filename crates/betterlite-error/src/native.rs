use crate::anthropic_replicate_errors::anthropic::{self, AnthropicExceptionInput};
use crate::anthropic_replicate_errors::replicate::{self, ReplicateExceptionInput};
use crate::aws_errors::bedrock::{self, BedrockExceptionInput};
use crate::aws_errors::sagemaker::{self, SagemakerExceptionInput};
use crate::azure_openrouter::azure::body::{AzureErrorBody, AzureErrorDetail};
use crate::azure_openrouter::azure::{self, AzureExceptionInput};
use crate::azure_openrouter::openrouter::{self, OpenrouterExceptionInput};
use crate::dispatch::{RawException, exception_provider_label};
use crate::error::ErrorClass;
use crate::facts::ProviderErrorFacts;
use crate::native_errors_a::{ai21, cloudflare, cohere, huggingface};
use crate::native_errors_b::aleph_alpha::{self, AlephAlphaExceptionInput};
use crate::native_errors_b::nlp_cloud::{self, NlpCloudExceptionInput};
use crate::native_errors_b::ollama::{self, OllamaExceptionInput, OllamaOriginalException};
use crate::native_errors_b::together_ai::{self, TogetherAiExceptionInput};
use crate::native_errors_b::vllm::{self, VllmExceptionInput};
use crate::openai::body::OpenaiErrorBody;
use crate::{openai_like, vertex_errors};

mod tests;

// litellm._openai_like_providers, checked ahead of bedrock in exception_type()'s elif chain
const OPENAI_LIKE_PROVIDERS: &[&str] = &["predibase", "databricks", "lemonade", "watsonx"];

const fn facts(raw: &RawException, message: Option<String>) -> ProviderErrorFacts {
    ProviderErrorFacts {
        status_code: raw.status_code,
        status_code_is_synthesized: raw.status_code_is_synthesized,
        message,
        body: None,
        headers: None,
        response: None,
        litellm_response_headers: None,
        request: None,
    }
}

// inner_error/innererror are never captured by RawException's flattened body, so they stay None here
fn azure_body(body: Option<&OpenaiErrorBody>) -> Option<AzureErrorBody> {
    let fields = body?;
    Some(AzureErrorBody {
        kind: fields.kind.clone(),
        param: fields.param.clone(),
        code: fields.code.clone(),
        message: fields.message.clone(),
        error: fields.error.as_ref().map(|detail| AzureErrorDetail {
            kind: None,
            param: None,
            code: None,
            message: detail.message.clone(),
            inner_error: None,
            innererror: None,
        }),
    })
}

// exception_type()'s native-family elif chain, in its exact source order, falling through to the catchall
#[must_use]
pub fn dispatch_native(raw: &RawException, error_str: &str) -> Option<ErrorClass> {
    let provider = raw.custom_llm_provider.as_str();
    match provider {
        "anthropic" | "anthropic_text" | "replicate" => {
            dispatch_anthropic_replicate(raw, error_str, provider)
        }
        _ if OPENAI_LIKE_PROVIDERS.contains(&provider) => openai_like::map_openai_like_exception(
            &facts(raw, None),
            error_str,
            provider,
            Some(raw.model.clone()),
        ),
        "bedrock" | "bedrock_mantle" | "sagemaker" | "sagemaker_chat" => {
            dispatch_aws(raw, error_str, provider)
        }
        "vertex_ai" | "vertex_ai_beta" | "gemini" => vertex_errors::map_vertex_exception(
            &facts(raw, None),
            error_str,
            provider,
            Some(raw.model.clone()),
        ),
        "cloudflare" | "cohere" | "cohere_chat" | "huggingface" | "ai21" => {
            dispatch_native_a(raw, error_str, provider)
        }
        "nlp_cloud" | "together_ai" | "aleph_alpha" | "ollama" | "ollama_chat" | "vllm" => {
            dispatch_native_b(raw, error_str, provider)
        }
        "azure" | "azure_text" | "openrouter" => {
            Some(dispatch_azure_openrouter(raw, error_str, provider))
        }
        _ => None,
    }
}

fn dispatch_anthropic_replicate(
    raw: &RawException,
    error_str: &str,
    provider: &str,
) -> Option<ErrorClass> {
    match provider {
        "anthropic" | "anthropic_text" => {
            anthropic::map_anthropic_exception(&AnthropicExceptionInput {
                model: raw.model.clone(),
                error_str: error_str.to_owned(),
                status_code: raw.status_code,
            })
        }
        _ => Some(replicate::map_replicate_exception(
            &ReplicateExceptionInput {
                model: raw.model.clone(),
                error_str: error_str.to_owned(),
                exception_type: raw.exception_type.clone(),
                status_code: raw.status_code,
            },
        )),
    }
}

fn dispatch_aws(raw: &RawException, error_str: &str, provider: &str) -> Option<ErrorClass> {
    match provider {
        "bedrock" | "bedrock_mantle" => bedrock::map_bedrock_exception(&BedrockExceptionInput {
            model: raw.model.clone(),
            custom_llm_provider: raw.custom_llm_provider.clone(),
            error_str: error_str.to_owned(),
            status_code: raw.status_code,
        }),
        _ => sagemaker::map_sagemaker_exception(&SagemakerExceptionInput {
            model: raw.model.clone(),
            custom_llm_provider: raw.custom_llm_provider.clone(),
            error_str: error_str.to_owned(),
            status_code: raw.status_code,
        }),
    }
}

fn dispatch_native_a(raw: &RawException, error_str: &str, provider: &str) -> Option<ErrorClass> {
    match provider {
        "cloudflare" => cloudflare::map_cloudflare_exception(error_str, Some(raw.model.clone())),
        "cohere" | "cohere_chat" => cohere::map_cohere_exception(
            &facts(raw, None),
            error_str,
            &raw.exception_type,
            Some(raw.model.clone()),
        ),
        "huggingface" => huggingface::map_huggingface_exception(
            &facts(raw, None),
            error_str,
            Some(raw.model.clone()),
        ),
        // dead in the full dispatch() pipeline: is_openai_family() already intercepts "ai21"/"ai21_chat"
        _ => ai21::map_ai21_exception(&facts(raw, raw.message.clone()), Some(raw.model.clone())),
    }
}

fn dispatch_native_b(raw: &RawException, error_str: &str, provider: &str) -> Option<ErrorClass> {
    match provider {
        "nlp_cloud" => nlp_cloud::map_nlp_cloud_exception(&NlpCloudExceptionInput {
            model: raw.model.clone(),
            error_str: error_str.to_owned(),
            status_code: raw.status_code,
            message: raw.message.clone(),
        }),
        // dead in the full dispatch() pipeline, for the same reason as "ai21" above
        "together_ai" => together_ai::map_together_ai_exception(&TogetherAiExceptionInput {
            model: raw.model.clone(),
            error_str: error_str.to_owned(),
            status_code: raw.status_code,
            message: raw.message.clone(),
        }),
        "aleph_alpha" => aleph_alpha::map_aleph_alpha_exception(&AlephAlphaExceptionInput {
            model: raw.model.clone(),
            error_str: error_str.to_owned(),
            status_code: raw.status_code,
            message: raw.message.clone(),
        }),
        // original_exception is never a dict once it has already been flattened into RawException's string fields
        "ollama" | "ollama_chat" => ollama::map_ollama_exception(&OllamaExceptionInput {
            model: raw.model.clone(),
            original_exception: OllamaOriginalException::Text {
                text: error_str.to_owned(),
            },
        }),
        _ => vllm::map_vllm_exception(&VllmExceptionInput {
            model: raw.model.clone(),
            status_code: raw.status_code,
            message: raw.message.clone(),
        }),
    }
}

fn dispatch_azure_openrouter(raw: &RawException, error_str: &str, provider: &str) -> ErrorClass {
    match provider {
        "azure" | "azure_text" => azure::map_azure_exception(&AzureExceptionInput {
            model: raw.model.clone(),
            custom_llm_provider: raw.custom_llm_provider.clone(),
            error_str: error_str.to_owned(),
            exception_provider: exception_provider_label(&raw.custom_llm_provider),
            status_code: raw.status_code,
            message: raw.message.clone(),
            body: azure_body(raw.body.as_ref()),
        }),
        _ => openrouter::map_openrouter_exception(&OpenrouterExceptionInput {
            model: raw.model.clone(),
            custom_llm_provider: raw.custom_llm_provider.clone(),
            error_str: error_str.to_owned(),
            exception_provider: exception_provider_label(&raw.custom_llm_provider),
            status_code: raw.status_code,
            status_code_is_synthesized: raw.status_code_is_synthesized,
        }),
    }
}
