use std::collections::BTreeMap;

use rust_decimal::Decimal;
use serde::Serialize;

use super::model::{ExceptionInfo, Inputs, OptionalParams, Payload, Response};

#[derive(Clone, Debug, Default, Serialize)]
pub struct Kwargs<Data> {
    pub litellm_call_id: Option<String>,
    pub call_type: Option<String>,
    pub model: Option<String>,
    pub stream: bool,
    pub messages: Option<Vec<Data>>,
    pub tools: Option<Vec<Data>>,
    pub prediction: Option<Data>,
    pub optional_params: OptionalParams<Data>,
    pub standard_logging_object: Option<Standard<Data>>,
    pub cache_hit: Option<bool>,
    pub custom_llm_provider: Option<String>,
    pub litellm_params: ProviderParams,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub response_cost: Option<Decimal>,
    pub exception: Option<ExceptionInfo>,
    pub complete_streaming_response: Option<Response<Data>>,
    #[serde(rename = "async_complete_streaming_response")]
    pub async_complete_response: Option<Response<Data>>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ProviderParams {
    pub api_base: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Standard<Data> {
    pub api_base: Option<String>,
    pub cache_hit: Option<bool>,
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub response: Option<Data>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub response_cost: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub saved_cache_cost: Option<Decimal>,
    pub request_tags: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Attributes<Data> {
    pub litellm_call_id: Option<String>,
    pub call_type: Option<String>,
    pub model: Option<String>,
    #[serde(flatten)]
    pub details: Details<Data>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Details<Data> {
    Standard {
        api_base: Option<String>,
        cache_hit: Option<bool>,
        #[serde(rename = "mlflow.chat.tokenUsage")]
        usage: Usage,
        raw_llm_response: Option<Data>,
        #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
        response_cost: Option<Decimal>,
        #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
        saved_cache_cost: Option<Decimal>,
        request_tags: Vec<String>,
    },
    Fallback {
        api_base: Option<String>,
        cache_hit: Option<bool>,
        custom_llm_provider: Option<String>,
        #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
        response_cost: Option<Decimal>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Usage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

impl<Data: Payload> Kwargs<Data> {
    pub fn construct_input(&mut self) -> Inputs<Data> {
        let optional = &mut self.optional_params;
        let tools = optional.tools.take().filter(|items| !items.is_empty());
        Inputs {
            messages: self.messages.clone(),
            tools: tools.or_else(|| self.tools.clone().filter(|items| !items.is_empty())),
            functions: optional.functions.take().filter(|items| !items.is_empty()),
            stream: optional.stream.take().filter(|value| *value),
            tool_choice: optional.tool_choice.take().filter(Payload::is_truthy),
            user: optional.user.take().filter(|value| !value.is_empty()),
            prediction: self.prediction.clone().filter(Payload::is_truthy),
        }
    }

    pub fn extract_attributes(&self) -> Attributes<Data> {
        let details = self.standard_logging_object.as_ref().map_or_else(
            || Details::Fallback {
                api_base: self.litellm_params.api_base.clone(),
                cache_hit: self.cache_hit,
                custom_llm_provider: self.custom_llm_provider.clone(),
                response_cost: self.response_cost,
            },
            |standard| Details::Standard {
                api_base: standard.api_base.clone(),
                cache_hit: standard.cache_hit,
                usage: Usage {
                    input_tokens: standard.prompt_tokens,
                    output_tokens: standard.completion_tokens,
                    total_tokens: standard.total_tokens,
                },
                raw_llm_response: standard.response.clone(),
                response_cost: standard.response_cost,
                saved_cache_cost: standard.saved_cache_cost,
                request_tags: standard.request_tags.clone(),
            },
        );
        Attributes {
            litellm_call_id: self.litellm_call_id.clone(),
            call_type: self.call_type.clone(),
            model: self.model.clone(),
            details,
        }
    }
}

#[must_use]
pub fn transform_tags(tags: &[String]) -> BTreeMap<String, String> {
    let mut result = BTreeMap::new();
    for tag in tags {
        let (key, value) = tag.split_once(':').unwrap_or((tag, ""));
        result.insert(key.trim().to_owned(), value.trim().to_owned());
    }
    result
}
