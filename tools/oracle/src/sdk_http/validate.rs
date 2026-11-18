use betterlite_models::decisions::Json;

use super::call_checks::validate_timeout;
use super::error::SdkHttpError;
use super::messages_chat_stream;
use super::messages_sync_stream::call;
use super::{OracleLimits, SdkHttpCall, SdkHttpCase};

// the harness owns the child's proxy routing, interpreter startup and loader paths, so a case never sets them
fn reserved_environment(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    matches!(
        upper.as_str(),
        "ALL_PROXY"
            | "HOME"
            | "HTTP_PROXY"
            | "HTTPS_PROXY"
            | "LITELLM_LOCAL_MODEL_COST_MAP"
            | "NO_PROXY"
            | "PATH"
    ) || upper.starts_with("PYTHON")
        || upper.starts_with("UV_")
}

fn validate_call(call: &SdkHttpCall, limits: &OracleLimits) -> Result<(), SdkHttpError> {
    match call {
        SdkHttpCall::OpenaiChat(chat) => validate_timeout(chat.timeout_seconds, limits)?,
        SdkHttpCall::VectorStoreEmbeddingExecutor(embedding) => {
            for deployment in embedding.router.iter().flatten() {
                if !deployment.litellm_params.model.starts_with("openai/") {
                    return Err(SdkHttpError::Invalid(
                        "every router deployment must serve an openai/ model".to_owned(),
                    ));
                }
            }
        }
        SdkHttpCall::MessagesHeaderStream(stream) => {
            if !matches!(stream.body, Json::Object(_)) {
                return Err(SdkHttpError::Invalid(
                    "the Messages header stream body must be an object".to_owned(),
                ));
            }
        }
        SdkHttpCall::MessagesStreamFactory(factory) => {
            if factory.model.is_empty() {
                return Err(SdkHttpError::Invalid(
                    "the Messages factory call needs a model name".to_owned(),
                ));
            }
            if !matches!(factory.body, Json::Object(_)) {
                return Err(SdkHttpError::Invalid(
                    "the Messages factory request body must be an object".to_owned(),
                ));
            }
        }
        SdkHttpCall::MessagesNativeStream(relay) => {
            if relay.queue_maxsize == 0 {
                return Err(SdkHttpError::Invalid(
                    "the relay queue maxsize must be greater than 0".to_owned(),
                ));
            }
        }
        SdkHttpCall::MessagesSyncStream(stream) => call::validate(stream, limits)?,
        SdkHttpCall::MessagesChatStream(stream) => {
            messages_chat_stream::call::validate(stream, limits)?;
        }
        SdkHttpCall::ChatResponseStream(stream) => {
            validate_timeout(stream.timeout_seconds, limits)?;
        }
        SdkHttpCall::ResponsesAgenticContinuation(_)
        | SdkHttpCall::VectorQueryEmbedding(_)
        | SdkHttpCall::AnthropicBetaConfig(_)
        | SdkHttpCall::MessagesStreamLogging(_)
        | SdkHttpCall::FileAzureStorage(_) => {}
    }
    Ok(())
}

pub(super) fn validate(case: &SdkHttpCase, limits: &OracleLimits) -> Result<(), SdkHttpError> {
    let invalid = |message: &str| Err(SdkHttpError::Invalid(message.to_owned()));
    if case.kind != "sdk_http" || case.name.is_empty() {
        return invalid("the case needs kind sdk_http and a name");
    }
    if let Some(name) = case.env.keys().find(|name| reserved_environment(name)) {
        return invalid(&format!("the case env may not set {name}"));
    }
    validate_call(&case.call, limits)?;
    if case.upstream.len() > limits.max_requests {
        return invalid("the case scripts more provider answers than the request limit");
    }
    for answer in &case.upstream {
        if answer.body.is_some() == answer.events.is_some() {
            return invalid("an upstream answer needs exactly one of body or events");
        }
        if !(100..=599).contains(&answer.status) {
            return invalid("an upstream answer has an invalid status");
        }
    }
    if !case.compare.headers.is_empty() || !case.compare.headers_present.is_empty() {
        return invalid("compare.headers and compare.headers_present describe gateway responses");
    }
    for ignored in &case.compare.ignore {
        if !ignored.pointer.is_empty() && !ignored.pointer.starts_with('/') {
            return invalid("compare.ignore pointers must be JSON pointers");
        }
        if ignored.why.is_empty() {
            return invalid("compare.ignore entries need a why");
        }
    }
    Ok(())
}
