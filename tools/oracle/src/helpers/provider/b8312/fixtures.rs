use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::sections::regional_host_keys::{Environment, Settings as SectionSettings};
use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::embedding::EmbeddingInput;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_prompts::arguments::json::Object;
use betterlite_provider::hosted_platform_dispatch::call::{CompletionCall, EmbeddingCall};
use betterlite_provider::hosted_platform_dispatch::handler::{ChatCompletionHandler, HandlerError};
use betterlite_provider::regional_host_dispatch::chat;
use betterlite_provider::regional_host_dispatch::context::{
    CallLogging, DispatchContext, EmbeddingContext,
};
use betterlite_provider::regional_host_dispatch::embedding;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::oracle::Case;

mod speech;
mod views;

pub(super) use speech::SpeechRecorder;
pub(crate) use views::{dashscope_view, minimax_chat_view, nlp_cloud_view, volcengine_view};

const INIT: &str = r#"{"model":"oracle","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
// the two headers both sides build themselves; the HTTP client adds the rest
const KEPT: [&str; 2] = ["authorization", "content-type"];

pub(super) fn input_failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("ValueError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

pub(super) fn kept<'name, Pairs>(pairs: Pairs) -> BTreeMap<String, String>
where
    Pairs: Iterator<Item = (&'name String, &'name String)>,
{
    pairs
        .filter(|(name, _)| KEPT.contains(&name.to_lowercase().as_str()))
        .map(|(name, value)| (name.to_lowercase(), value.clone()))
        .collect()
}

// ast-grep-ignore: no-one-line-functions - every view in both files builds this same request shape
pub(super) fn request_view(url: &str, headers: &BTreeMap<String, String>) -> Value {
    json!({"url": url, "headers": headers})
}

// every key is named; the variables the cases set are read and the rest stay unset
#[rustfmt::skip]
pub(super) fn section_settings(case: &Case) -> SectionSettings {
    let read = |name: &str| case.env.get(name).filter(|text| !text.is_empty()).cloned();
    let nonempty = |value: &Option<String>| value.clone().filter(|text| !text.is_empty());
    SectionSettings {
        api_base: nonempty(&case.litellm_api_base),
        api_key: nonempty(&case.litellm_api_key).map(SecretString::from),
        headers: None,
        gigachat_key: nonempty(&case.gigachat_key).map(SecretString::from),
        nlp_cloud_key: nonempty(&case.nlp_cloud_key).map(SecretString::from),
        environment: Environment {
            ark_api_key: read("ARK_API_KEY").map(SecretString::from), ark_api_base: None,
            dashscope_api_key: read("DASHSCOPE_API_KEY").map(SecretString::from), dashscope_base: read("DASHSCOPE_API_BASE"),
            dashscope_rerank_base: None, dashscope_image_base: None, modelscope_api_base: None, modelscope_api_key: None,
            gigachat_api_key: read("GIGACHAT_API_KEY").map(SecretString::from), gigachat_credentials: read("GIGACHAT_CREDENTIALS").map(SecretString::from),
            minimax_api_key: read("MINIMAX_API_KEY").map(SecretString::from), minimax_api_base: None,
            nlp_cloud_api_key: read("NLP_CLOUD_API_KEY").map(SecretString::from), nlp_cloud_api_base: read("NLP_CLOUD_API_BASE"),
            qwen_ai_platform_key: None, qwen_ai_platform_base: None, qwen_ai_platform_rerank_base: None, qwen_ai_platform_image_base: None,
            qwencloud_key: None, qwencloud_base: None, qwencloud_rerank_base: None, qwencloud_image_base: None,
            tencent_api_base: None, tencent_api_key: None, volcengine_api_base: None, volcengine_api_key: None,
        },
    }
}

pub(super) fn chat_call(
    case: &Case,
    tag: ChatConfigTag,
    provider: &'static str,
) -> Result<CompletionCall, Box<HelperError>> {
    let settings = section_settings(case);
    let optional_params = Object::new();
    let litellm_params = Object::new();
    let api_key = case.api_key.as_deref().map(SecretString::from);
    let context = DispatchContext {
        acompletion: false,
        api_base: case.api_base.as_deref(),
        api_key: api_key.as_ref(),
        custom_llm_provider: provider,
        headers: None,
        litellm_params: &litellm_params,
        messages: &[],
        model: &case.model,
        optional_params: &optional_params,
        provider_config: Some(tag),
        stream: None,
        timeout: None,
        settings: &settings,
    };
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_value_error()?;
    let (mut state, prepost) = logging(&redactor)?;
    let mut log = CallLogging {
        state: &mut state,
        env: &prepost,
    };
    let handler = ChatRecorder::default();
    chat::dispatch(tag, &context, &handler, &mut log)
        .ok_or_else(|| input_failure(format!("{provider} has no chat arm")))?
        .map_err(|error| input_failure(error.to_string()))?;
    handler.first()
}

pub(super) fn embedding_call(
    case: &Case,
    provider: ProviderId,
) -> Result<EmbeddingCall, Box<HelperError>> {
    let settings = section_settings(case);
    let input = EmbeddingInput::Text(case.input.clone());
    let optional_params = Object::new();
    let api_key = case.api_key.as_deref().map(SecretString::from);
    let context = EmbeddingContext {
        aembedding: false,
        api_base: case.api_base.as_deref(),
        api_key: api_key.as_ref(),
        extra_headers: None,
        input: &input,
        model: &case.model,
        optional_params: &optional_params,
        ssl_verify: None,
        timeout: Duration::from_secs(600),
        settings: &settings,
    };
    embedding::dispatch(provider, &context)
        .ok_or_else(|| input_failure(String::from("the provider has no embedding arm")))?
        .map_err(|error| input_failure(error.to_string()))
}

struct Discard;

impl LogSink for Discard {
    fn emit(&self, _record: LogRecord) {}
}

pub(super) fn logging(
    redactor: &Redactor,
) -> Result<(LoggingState, PrePostEnv<'_>), Box<HelperError>> {
    let init = serde_json::from_str::<LoggingInit>(INIT).or_value_error()?;
    let state = LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("trace"),
        Duration::default,
    )
    .or_value_error()?;
    let env = PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags {
            log_raw_request_response: true,
            turn_off_message_logging: false,
        },
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: &Discard,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").or_value_error()?,
    };
    Ok((state, env))
}

#[derive(Default)]
pub(super) struct ChatRecorder {
    calls: Mutex<Vec<CompletionCall>>,
}

impl ChatRecorder {
    pub(super) fn first(&self) -> Result<CompletionCall, Box<HelperError>> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .first()
            .cloned()
            .ok_or_else(|| input_failure(String::from("the chat arm called no handler")))
    }
}

impl ChatCompletionHandler for ChatRecorder {
    type Reply = ();

    fn complete(&self, call: &CompletionCall) -> Result<Self::Reply, HandlerError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call.clone());
        Ok(())
    }
}
