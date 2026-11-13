use std::convert::Infallible;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::sections::local_ttl_cache::Settings;
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_context::local_ttl_cache::dual::{Diagnostics, DualLocalCache, Ttl};
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::responses_chat_options::capabilities::CompletionBridgeProbe;
use betterlite_core::api::responses_chat_options::records::{ChatCompletionRequest, StreamOptions};
use betterlite_core::api::responses_chat_options::{
    RequestAssembly, RequestContext, SUPPORTED_RESPONSES_OPTIONS, StreamOptionsSink,
    chat_completion_request,
};
use betterlite_core::responses_bridge::{BridgeRequest, ModelInfoError, ReasoningEffort};
use betterlite_models::decisions::Json;
use betterlite_models::full_model_metadata::parameters::SupportedParameterSource;
use betterlite_openai::api::responses_objects::request_params::ResponsesApiRequestParams;
use betterlite_redact::error::RedactionError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::helpers::OrHelperError as _;

#[derive(Debug, Deserialize)]
pub(crate) struct CacheEntry {
    key: String,
    value: CacheValue,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    request: ResponsesApiRequestParams,
    provider: Option<String>,
    stream: Option<bool>,
    extra_headers: Option<Json>,
    metadata: Option<Json>,
    service_tier: Option<String>,
    api_base: Option<String>,
    supported_openai_params: Option<Vec<String>>,
    bridge_mode: Option<String>,
    bridge_failure: Option<String>,
    route_all_openai_to_responses: bool,
    cache: Vec<CacheEntry>,
}

#[derive(Debug, Serialize)]
struct ParameterCall {
    model: String,
    custom_llm_provider: Option<String>,
}

#[derive(Debug, Serialize)]
struct ProbeCall {
    model: String,
    custom_llm_provider: String,
    web_search_options: Option<Value>,
    tools: Vec<Value>,
    reasoning_effort: Option<String>,
    reasoning_summary: Option<Value>,
    api_base: Option<String>,
}

struct Parameters {
    supported: Option<Vec<String>>,
    calls: Mutex<Vec<ParameterCall>>,
}

impl Parameters {
    fn calls(&self) -> Vec<ParameterCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
            .collect()
    }
}

impl SupportedParameterSource for Parameters {
    type Error = Infallible;

    fn supported_openai_params(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Result<Option<Vec<String>>, Infallible> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ParameterCall {
                model: model.to_owned(),
                custom_llm_provider: custom_llm_provider.map(str::to_owned),
            });
        Ok(self.supported.clone())
    }
}

struct Probe {
    mode: Option<String>,
    failure: Option<String>,
    calls: Mutex<Vec<ProbeCall>>,
}

impl Probe {
    fn calls(&self) -> Vec<ProbeCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
            .collect()
    }
}

impl CompletionBridgeProbe for Probe {
    fn bridges_back(&self, request: &BridgeRequest<'_>) -> Result<bool, ModelInfoError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ProbeCall {
                model: request.model.to_owned(),
                custom_llm_provider: request.custom_llm_provider.to_owned(),
                web_search_options: request.web_search_options.cloned(),
                tools: request.tools.to_vec(),
                reasoning_effort: request.reasoning_effort.and_then(|effort| match effort {
                    ReasoningEffort::Text(text) => Some(text.clone()),
                    ReasoningEffort::Object(_) => None,
                }),
                reasoning_summary: request.reasoning_summary.cloned(),
                api_base: request.api_base.map(str::to_owned),
            });
        self.failure.as_ref().map_or_else(
            || Ok(self.mode.as_deref() == Some("responses")),
            |message| {
                Err(ModelInfoError::Unknown {
                    model: message.clone(),
                })
            },
        )
    }
}

#[derive(Default)]
struct RecordingSink {
    options: Option<StreamOptions>,
}

impl StreamOptionsSink for RecordingSink {
    fn set_stream_options(&mut self, options: &StreamOptions) {
        self.options = Some(*options);
    }
}

#[derive(Serialize)]
struct Output {
    request: ChatCompletionRequest,
    supported_options: [&'static str; 15],
    parameters: Vec<ParameterCall>,
    probe: Vec<ProbeCall>,
    logged: Option<StreamOptions>,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let cache = tool_cache(&input.cache).or_value_error()?;
        let parameters = Parameters {
            supported: input.supported_openai_params.clone(),
            calls: Mutex::new(Vec::new()),
        };
        let probe = Probe {
            mode: input.bridge_mode.clone(),
            failure: input.bridge_failure.clone(),
            calls: Mutex::new(Vec::new()),
        };
        let mut logging = RecordingSink::default();
        let mut assembly = RequestAssembly {
            limits: ApiLimits::default(),
            cache: &cache,
            parameters: &parameters,
            probe: &probe,
            logging: Some(&mut logging),
            route_all_openai_to_responses: input.route_all_openai_to_responses,
        };
        let context = RequestContext {
            request: &input.request,
            provider: input.provider.as_deref(),
            stream: input.stream,
            extra_headers: input.extra_headers.as_ref(),
            metadata: input.metadata.as_ref(),
            service_tier: input.service_tier.as_deref(),
            api_base: input.api_base.as_deref(),
        };
        let request = chat_completion_request(&context, &mut assembly).or_value_error()?;
        let output = Output {
            request,
            supported_options: SUPPORTED_RESPONSES_OPTIONS,
            parameters: parameters.calls(),
            probe: probe.calls(),
            logged: logging.options,
        };
        serde_json::to_value(output).or_value_error()
    }
}

fn tool_cache(entries: &[CacheEntry]) -> Result<DualLocalCache, RedactionError> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)?;
    let cache = DualLocalCache::new(
        None,
        None,
        &Settings::default(),
        Arc::new(|| 0.0_f64),
        Diagnostics {
            redactor: Arc::new(redactor),
            emit: Arc::new(|_event| {}),
        },
    );
    for entry in entries {
        cache.set(&entry.key, entry.value.clone(), Ttl::Absent);
    }
    Ok(cache)
}
