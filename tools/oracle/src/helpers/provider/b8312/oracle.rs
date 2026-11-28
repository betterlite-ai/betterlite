use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::regional_host_keys::{Environment, Settings as SectionSettings};
use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_models::decisions::Json as DecisionsJson;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::api::speech_provider_contract::SpeechVoice;
use betterlite_openai::dashscope_embedding;
use betterlite_openai::dashscope_family::{self, DashscopeFamily, FamilyEnv};
use betterlite_openai::embedding::EmbeddingInput;
use betterlite_openai::minimax_chat;
use betterlite_openai::nlp_cloud_chat::headers::validate_environment as nlp_headers;
use betterlite_openai::openai_base_config::Object as WireObject;
use betterlite_openai::volcengine_embedding::request;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_prompts::arguments::json::Object;
use betterlite_provider::hosted_platform_dispatch::call::{CompletionCall, EmbeddingCall};
use betterlite_provider::regional_host_dispatch::chat;
use betterlite_provider::regional_host_dispatch::context::{
    CallLogging, DispatchContext, EmbeddingContext,
};
use betterlite_provider::regional_host_dispatch::embedding;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

use super::fixtures::{ChatRecorder, input_failure, kept, logging, request_view};

// the litellm globals, the request values and the variables each arm falls back to
#[derive(Deserialize, Default)]
#[serde(default)]
pub(crate) struct Case {
    op: String,
    model: String,
    input: String,
    voice: Option<SpeechVoice>,
    api_base: Option<String>,
    api_key: Option<String>,
    litellm_api_base: Option<String>,
    litellm_api_key: Option<String>,
    gigachat_key: Option<String>,
    nlp_cloud_key: Option<String>,
    env: BTreeMap<String, String>,
}

pub(crate) fn run(case: &Case) -> HelperResult {
    match case.op.as_str() {
        "minimax_chat" => minimax_chat_view(case),
        "nlp_cloud_chat" => nlp_cloud_view(case),
        "gigachat_chat" => {
            let key = chat_call(case, ChatConfigTag::GigaChat, "gigachat")?.api_key;
            Ok(json!({"credentials": key.map(|secret| secret.expose_secret().to_owned())}))
        }
        "gigachat_embedding" => {
            let key = embedding_call(case, ProviderId::Gigachat)?.api_key;
            Ok(json!({"credentials": key.map(|secret| secret.expose_secret().to_owned())}))
        }
        "volcengine_embedding" => volcengine_view(case),
        "dashscope_embedding" => dashscope_view(case),
        "minimax_speech" => super::speech::view(case),
        other => Err(input_failure(format!("unknown op {other}"))),
    }
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

fn chat_call(
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

fn minimax_chat_view(case: &Case) -> HelperResult {
    let call = chat_call(case, ChatConfigTag::MinimaxChat, "minimax")?;
    let mut headers = BTreeMap::from([(
        String::from("content-type"),
        String::from("application/json"),
    )]);
    if let Some(key) = call.api_key.as_ref() {
        headers.insert(
            String::from("authorization"),
            format!("Bearer {}", key.expose_secret()),
        );
    }
    let url = minimax_chat::complete_url(call.api_base.as_deref(), None);
    Ok(json!({"request": request_view(&url, &headers)}))
}

fn nlp_cloud_view(case: &Case) -> HelperResult {
    let call = chat_call(case, ChatConfigTag::NlpCloud, "nlp_cloud")?;
    let base = call
        .api_base
        .as_deref()
        .ok_or_else(|| input_failure(String::from("the nlp_cloud arm resolved no api_base")))?;
    let headers = nlp_headers(&BTreeMap::new(), call.api_key.as_ref());
    let url = format!("{base}{}/generation", call.model);
    Ok(json!({"request": request_view(&url, &kept(headers.iter()))}))
}

fn embedding_call(case: &Case, provider: ProviderId) -> Result<EmbeddingCall, Box<HelperError>> {
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

fn volcengine_view(case: &Case) -> HelperResult {
    let call = embedding_call(case, ProviderId::Volcengine)?;
    let caller = call.headers.clone().unwrap_or_default();
    let headers = request::validate_environment(&caller, call.api_key.as_ref()).or_value_error()?;
    let body = request::transform_embedding_request(&call.model, &call.input, &WireObject::new());
    let url = request::get_complete_url(call.api_base.as_deref());
    Ok(json!({
        "request": request_view(&url, &kept(headers.iter())),
        "params": Value::Object(body),
    }))
}

fn dashscope_view(case: &Case) -> HelperResult {
    let call = embedding_call(case, ProviderId::Dashscope)?;
    let settings = section_settings(case);
    let env = FamilyEnv {
        base: settings.environment.dashscope_base.clone(),
        brand_key: None,
        fallback_key: settings.environment.dashscope_api_key.clone(),
        rerank_base: None,
        image_base: None,
    };
    let config = dashscope_family::embedding_config(DashscopeFamily::Dashscope, &env);
    let key = call.api_key.as_ref().map(|key| key.expose_secret());
    let caller = call.headers.clone().unwrap_or_default();
    let headers = config.validate_environment(key, &caller).or_value_error()?;
    let body = dashscope_embedding::transform_embedding_request(
        &call.model,
        &DecisionsJson::Text(case.input.clone()),
        &IndexMap::new(),
    );
    let url = config.complete_url(call.api_base.as_deref());
    Ok(json!({
        "request": request_view(&url, &kept(headers.iter())),
        "params": serde_json::to_value(body)?,
    }))
}
