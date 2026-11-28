use std::collections::BTreeMap;

use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::api::speech_provider_contract::SpeechVoice;
use betterlite_oracle_helper_api::helper::HelperResult;
use secrecy::ExposeSecret as _;
use serde::Deserialize;
use serde_json::json;

use super::fixtures::{
    chat_call, dashscope_view, embedding_call, input_failure, minimax_chat_view, nlp_cloud_view,
    volcengine_view,
};

// the litellm globals, the request values and the variables each arm falls back to
#[derive(Deserialize, Default)]
#[serde(default)]
pub(crate) struct Case {
    pub(super) op: String,
    pub(super) model: String,
    pub(super) input: String,
    pub(super) voice: Option<SpeechVoice>,
    pub(super) api_base: Option<String>,
    pub(super) api_key: Option<String>,
    pub(super) litellm_api_base: Option<String>,
    pub(super) litellm_api_key: Option<String>,
    pub(super) gigachat_key: Option<String>,
    pub(super) nlp_cloud_key: Option<String>,
    pub(super) env: BTreeMap<String, String>,
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
