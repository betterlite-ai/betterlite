use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::future::{Future, ready};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::generic_guardrail_contracts::GuardrailMessage;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StreamTransformSink {
    pub mutated_text_per_choice: IndexMap<i64, String>,
    pub holdback_per_choice: IndexMap<i64, i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamingScanKey {
    pub texts: Vec<String>,
    #[serde(default)]
    pub tool_calls: Vec<String>,
    #[serde(default)]
    pub stream_ended: bool,
    #[serde(default)]
    pub tool_calls_in_flight: bool,
}

impl PartialEq for StreamingScanKey {
    fn eq(&self, other: &Self) -> bool {
        self.texts == other.texts && self.tool_calls == other.tool_calls
    }
}

impl Eq for StreamingScanKey {}

impl StreamingScanKey {
    #[must_use]
    pub fn has_nothing_to_scan(&self) -> bool {
        if self.stream_ended || !self.tool_calls.is_empty() {
            return false;
        }
        self.texts.iter().all(String::is_empty)
    }
}

pub struct TranslationCall<'call, LogTarget> {
    pub user_api_key_dict: Option<&'call BTreeMap<String, GuardrailValue>>,
    pub request_data: Option<&'call mut BTreeMap<String, GuardrailValue>>,
    pub logging_obj: Option<&'call mut LogTarget>,
    pub stream_transform_sink: Option<&'call mut StreamTransformSink>,
    pub deliver_ended_stream_rewrites: bool,
}

// Bedrock's check vocabulary (LiteLLM's BedrockChecksContentFilterCategory, BedrockChecksPromptAttackCategory, BedrockChecksSensitiveInformationEntity) is config data in betterlite_config::sections::guardrail_translation
pub trait BaseTranslation {
    type Guardrail;
    type LogTarget;
    type Failure: StdError + Send + Sync + 'static;

    // LiteLLM's delivers_ended_stream_rewrites class flag
    const DELIVERS_ENDED_STREAM_REWRITES: bool = false;
    // LiteLLM's assembles_streamed_response class flag
    const ASSEMBLES_STREAMED_RESPONSE: bool = false;

    fn post_call_hook_response(&self, response: GuardrailValue) -> GuardrailValue {
        response
    }

    fn process_input_messages(
        &self,
        data: &mut BTreeMap<String, GuardrailValue>,
        guardrail: &Self::Guardrail,
        logging_obj: Option<&mut Self::LogTarget>,
    ) -> impl Future<Output = Result<(), Self::Failure>> + Send;

    fn process_output_response(
        &self,
        response: GuardrailValue,
        guardrail: &Self::Guardrail,
        call: TranslationCall<'_, Self::LogTarget>,
    ) -> impl Future<Output = Result<GuardrailValue, Self::Failure>> + Send;

    // LiteLLM's process_output_streaming_response; the name limit drops its output_ prefix
    fn process_streaming_response(
        &self,
        _responses_so_far: &mut Vec<GuardrailValue>,
        _guardrail: &Self::Guardrail,
        _call: TranslationCall<'_, Self::LogTarget>,
    ) -> impl Future<Output = Result<(), Self::Failure>> + Send {
        ready(Ok(()))
    }

    fn get_streaming_scan_key(
        &self,
        _responses_so_far: &[GuardrailValue],
    ) -> Option<StreamingScanKey> {
        None
    }

    fn build_block_sse_chunks(
        &self,
        _failure: &Self::Failure,
        _stream_started: bool,
        _responses_so_far: Option<&[GuardrailValue]>,
    ) -> Option<Vec<Vec<u8>>> {
        None
    }

    fn build_stream_error_items(
        &self,
        _failure: &Self::Failure,
        _responses_so_far: Option<&[GuardrailValue]>,
    ) -> Option<Vec<GuardrailValue>> {
        None
    }

    fn get_structured_messages(
        &self,
        _data: &BTreeMap<String, GuardrailValue>,
    ) -> Option<Vec<GuardrailMessage>> {
        None
    }

    // the empty default is an equivalent mutant: vec![] builds the same empty Vec
    #[cfg_attr(test, mutants::skip)]
    fn extract_request_tool_names(&self, _data: &BTreeMap<String, GuardrailValue>) -> Vec<String> {
        Vec::new()
    }
}

// LiteLLM's BaseTranslation.transform_user_api_key_dict_to_metadata; the name limit drops dict_
#[must_use]
pub fn transform_user_key_metadata(
    user_api_key_dict: Option<&BTreeMap<String, GuardrailValue>>,
) -> BTreeMap<String, GuardrailValue> {
    let Some(user_dict) = user_api_key_dict else {
        return BTreeMap::new();
    };
    let mut transformed = BTreeMap::new();
    for (key, value) in user_dict {
        if matches!(value, GuardrailValue::Null) || key.starts_with('_') {
            continue;
        }
        let metadata_key = if key.starts_with("user_api_key_") {
            key.clone()
        } else {
            format!("user_api_key_{key}")
        };
        transformed.insert(metadata_key, value.clone());
    }
    transformed
}

// LiteLLM's BaseTranslation.merge_user_api_key_metadata_into_request, shortened for the name limit
pub fn merge_user_key_metadata(
    request_data: &mut BTreeMap<String, GuardrailValue>,
    user_api_key_dict: Option<&BTreeMap<String, GuardrailValue>>,
) {
    let user_metadata = transform_user_key_metadata(user_api_key_dict);
    if user_metadata.is_empty() {
        return;
    }
    let bucket_name = if request_data.contains_key("litellm_metadata") {
        "litellm_metadata"
    } else {
        "metadata"
    };
    if !matches!(
        request_data.get(bucket_name),
        Some(GuardrailValue::Object(_))
    ) {
        request_data.insert(
            bucket_name.to_owned(),
            GuardrailValue::Object(BTreeMap::new()),
        );
    }
    let Some(GuardrailValue::Object(bucket)) = request_data.get_mut(bucket_name) else {
        return;
    };
    for (key, value) in user_metadata {
        bucket.entry(key).or_insert(value);
    }
}

pub mod cohere_rerank_guardrail;

mod tests;
