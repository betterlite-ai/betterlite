use betterlite_redact::secret::Redactor;

use crate::error::details::{TimeoutDetails, TimeoutStatus};
use crate::error::{ErrorClass, ErrorContext};
use crate::facts::ProviderErrorFacts;
use crate::openai::body::{OpenaiErrorBody, OpenaiErrorDetail};
use crate::openai::{self, OpenaiExceptionInput};
use crate::status::status_error_class;

mod litellm_proxy;
mod tests;

use litellm_proxy::{LitellmProxyOutcome, litellm_proxy_branch};

// exception_type()'s preamble passes an already-classified exception, or a guardrail block, straight through
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchInput {
    Mapped(ErrorClass),
    Raw(Box<RawException>),
}

// the subset of original_exception that exception_type()'s try block reads before any provider branch
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RawException {
    pub model: String,
    pub custom_llm_provider: String,
    pub error_str: String,
    pub status_code: Option<u16>,
    #[serde(default)]
    pub status_code_is_synthesized: bool,
    pub message: Option<String>,
    #[serde(default)]
    pub body: Option<OpenaiErrorBody>,
}

#[must_use]
pub fn dispatch(input: &DispatchInput, redactor: &Redactor) -> ErrorClass {
    match input {
        DispatchInput::Mapped(class) => class.clone(),
        DispatchInput::Raw(raw) => {
            let redacted = RawException {
                model: raw.model.clone(),
                custom_llm_provider: raw.custom_llm_provider.clone(),
                error_str: redactor.redact_string(&raw.error_str),
                status_code: raw.status_code,
                status_code_is_synthesized: raw.status_code_is_synthesized,
                message: raw
                    .message
                    .as_deref()
                    .map(|message| redactor.redact_string(message)),
                body: raw.body.as_ref().map(|body| OpenaiErrorBody {
                    kind: body.kind.clone(),
                    param: body.param.clone(),
                    code: body.code.clone(),
                    message: body
                        .message
                        .as_deref()
                        .map(|message| redactor.redact_string(message)),
                    error: body.error.as_ref().map(|detail| OpenaiErrorDetail {
                        message: detail
                            .message
                            .as_deref()
                            .map(|message| redactor.redact_string(message)),
                    }),
                }),
            };
            dispatch_raw(&redacted)
        }
    }
}

fn dispatch_raw(raw: &RawException) -> ErrorClass {
    // exception_type() overwrites error_str with str(original_exception.message) whenever that attribute exists
    let error_str = raw.message.as_deref().unwrap_or(&raw.error_str);
    // exception_type() nests every branch below under `if model or custom_llm_provider:`
    let has_model_or_provider = !raw.model.is_empty() || !raw.custom_llm_provider.is_empty();
    // the four literal phrases exception_type() checks before any provider-specific branch runs
    if has_model_or_provider
        && (error_str.contains("Request Timeout Error")
            || error_str.contains("Request timed out")
            || error_str.contains("Timed out generating response")
            || error_str.contains("The read operation timed out"))
    {
        return timeout_preamble_class(raw, error_str);
    }
    if has_model_or_provider && raw.custom_llm_provider == "litellm_proxy" {
        match litellm_proxy_branch(raw, error_str) {
            LitellmProxyOutcome::Raised(class) => return class,
            // not a regex match for "litellm.*Error": litellm_proxy also belongs to openai_compatible_providers
            LitellmProxyOutcome::NoMatch => {}
            // the matched name needs constructor arguments extract_and_raise_litellm_exception never supplies
            LitellmProxyOutcome::Unconstructible => return unmappable_identity(raw),
        }
    }
    if is_openai_family(&raw.custom_llm_provider) {
        // _map_openai_exception's kwargs come straight from a RawException once it is openai-family
        return openai::map_openai_exception(&OpenaiExceptionInput {
            model: raw.model.clone(),
            custom_llm_provider: raw.custom_llm_provider.clone(),
            error_str: error_str.to_owned(),
            status_code: raw.status_code,
            message: raw.message.clone(),
            body: raw.body.clone(),
        });
    }
    catchall(raw, error_str)
}

fn timeout_preamble_class(raw: &RawException, error_str: &str) -> ErrorClass {
    let context = ErrorContext {
        message: format!("APITimeoutError - Request timed out. Error_str: {error_str}"),
        model: Some(raw.model.clone()),
        provider: Some(raw.custom_llm_provider.clone()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    };
    ErrorClass::Timeout(Box::new(TimeoutDetails {
        context,
        original_status: TimeoutStatus(408),
    }))
}

// the generic except Exception as e: fallback once mapping itself fails with no exception_mapping_worked
fn unmappable_identity(raw: &RawException) -> ErrorClass {
    let context = ErrorContext {
        message: raw.error_str.clone(),
        model: Some(String::new()),
        provider: Some(String::new()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    };
    ErrorClass::ApiConnection(Box::new(context))
}

// the unconditional raise at the end of exception_type()'s try block, reached once no provider branch raises
fn catchall(raw: &RawException, error_str: &str) -> ErrorClass {
    let exception_provider = exception_provider_label(&raw.custom_llm_provider);
    // _map_exception_by_status runs first; a synthetic status never outranks this, since it bails out early
    let facts = ProviderErrorFacts {
        status_code: raw.status_code,
        status_code_is_synthesized: raw.status_code_is_synthesized,
        message: None,
        body: None,
        headers: None,
        response: None,
        litellm_response_headers: None,
        request: None,
    };
    if let Some(class) = status_error_class(
        &facts,
        error_str,
        &exception_provider,
        Some(raw.model.clone()),
        Some(raw.custom_llm_provider.clone()),
    ) {
        return class;
    }
    let context = ErrorContext {
        message: format!("{exception_provider} - {error_str}"),
        model: Some(raw.model.clone()),
        provider: Some(raw.custom_llm_provider.clone()),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    };
    ErrorClass::ApiConnection(Box::new(context))
}

// exception_provider = custom_llm_provider[0].upper() + custom_llm_provider[1:] + "Exception"
fn exception_provider_label(custom_llm_provider: &str) -> String {
    let mut chars = custom_llm_provider.chars();
    chars.next().map_or_else(
        || "Exception".to_owned(),
        |first| format!("{}{}Exception", first.to_uppercase(), chars.as_str()),
    )
}

const OPENAI_COMPATIBLE_PROVIDERS: &[&str] = &[
    "anyscale",
    "groq",
    "nvidia_nim",
    "cerebras",
    "baseten",
    "sambanova",
    "ai21_chat",
    "ai21",
    "volcengine",
    "codestral",
    "deepseek",
    "tencent",
    "deepinfra",
    "perplexity",
    "xinference",
    "xai",
    "zai",
    "together_ai",
    "fireworks_ai",
    "empower",
    "friendliai",
    "azure_ai",
    "github",
    "litellm_proxy",
    "hosted_vllm",
    "llamafile",
    "lm_studio",
    "galadriel",
    "github_copilot",
    "chatgpt",
    "novita",
    "meta_llama",
    "publicai",
    "synthetic",
    "tensormesh",
    "apertis",
    "nano-gpt",
    "poe",
    "chutes",
    "parasail",
    "libertai",
    "featherless_ai",
    "nscale",
    "nebius",
    "dashscope",
    "qwencloud",
    "qwen_ai_platform",
    "modelscope",
    "moonshot",
    "v0",
    "helicone",
    "morph",
    "lambda_ai",
    "inception",
    "hyperbolic",
    "vercel_ai_gateway",
    "aiml",
    "edenai",
    "wandb",
    "cometapi",
    "clarifai",
    "docker_model_runner",
    "ragflow",
    "pinstripes",
    "darkbloom",
    "meta",
    "cognition",
    "cortecs",
    "scx-ai",
    "prism",
    "sail",
];

// custom_llm_provider == "openai"/"text-completion-openai"/"custom_openai"/"mistral"/"runwayml", or litellm.openai_compatible_providers
fn is_openai_family(custom_llm_provider: &str) -> bool {
    match custom_llm_provider {
        "openai" | "text-completion-openai" | "custom_openai" | "mistral" | "runwayml" => true,
        other => OPENAI_COMPATIBLE_PROVIDERS.contains(&other),
    }
}
