#![cfg(test)]

use std::convert::Infallible;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::sections::local_ttl_cache::Settings;
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_context::local_ttl_cache::dual::{Diagnostics, DualLocalCache, Ttl};
use betterlite_models::decisions::Json;
use betterlite_models::full_model_metadata::parameters::SupportedParameterSource;
use betterlite_openai::api::responses_objects::request_params::ResponsesApiRequestParams;
use betterlite_openai::openai_base_config::Object;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use rstest::rstest;
use serde_json::Value;

use crate::api::api_boundaries::ApiLimits;
use crate::responses_bridge::{
    BridgeRequest, ModelInfoError, ReasoningEffort, ResponsesBridgeCapabilities,
};

use super::capabilities::{CompletionBridgeProbe, SdkBridgeProbe};
use super::error::ChatOptionsError;
use super::records::{ChatCompletionRequest, ChatCompletionSession, StreamOptions};
use super::{
    RequestAssembly, RequestContext, SUPPORTED_RESPONSES_OPTIONS, StreamOptionsSink,
    chat_completion_request,
};

const FUNCTION_TOOL: &str = r#"{"type":"function","name":"lookup","description":"Find","parameters":{"type":"object","properties":{}}}"#;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParameterCall {
    model: String,
    custom_llm_provider: Option<String>,
}

struct Parameters {
    supported: Option<Vec<String>>,
    calls: Mutex<Vec<ParameterCall>>,
}

impl Parameters {
    fn new(supported: Option<Vec<String>>) -> Self {
        Self {
            supported,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<ParameterCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProbeCall {
    model: String,
    custom_llm_provider: String,
    web_search_options: Option<Value>,
    tools: Vec<Value>,
    reasoning_effort: Option<String>,
    reasoning_summary: Option<Value>,
    api_base: Option<String>,
}

struct Probe {
    bridged: bool,
    failure: Option<String>,
    calls: Mutex<Vec<ProbeCall>>,
}

impl Probe {
    fn bridging(bridged: bool) -> Self {
        Self {
            bridged,
            failure: None,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn failing(message: &str) -> Self {
        Self {
            bridged: false,
            failure: Some(message.to_owned()),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<ProbeCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
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
        self.failure.as_ref().map_or(Ok(self.bridged), |message| {
            Err(ModelInfoError::Unknown {
                model: message.clone(),
            })
        })
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

struct Harness {
    cache: DualLocalCache,
    parameters: Parameters,
    probe: Probe,
    sink: RecordingSink,
}

impl Harness {
    fn new(supported: Option<Vec<String>>, bridged: bool) -> Self {
        Self {
            cache: cache(&[]),
            parameters: Parameters::new(supported),
            probe: Probe::bridging(bridged),
            sink: RecordingSink::default(),
        }
    }

    fn run(
        &mut self,
        context: &RequestContext<'_>,
    ) -> Result<ChatCompletionRequest, ChatOptionsError> {
        let mut assembly = RequestAssembly {
            limits: ApiLimits::default(),
            cache: &self.cache,
            parameters: &self.parameters,
            probe: &self.probe,
            logging: Some(&mut self.sink),
            route_all_openai_to_responses: false,
        };
        chat_completion_request(context, &mut assembly)
    }
}

fn cache(entries: &[(&str, &str)]) -> DualLocalCache {
    let diagnostics = Diagnostics {
        redactor: Arc::new(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap()),
        emit: Arc::new(|_event| {}),
    };
    let shared = DualLocalCache::new(
        None,
        None,
        &Settings::default(),
        Arc::new(|| 0.0_f64),
        diagnostics,
    );
    for (key, value) in entries {
        shared.set(
            key,
            serde_json::from_str::<CacheValue>(value).unwrap(),
            Ttl::Absent,
        );
    }
    shared
}

fn request(source: &str) -> ResponsesApiRequestParams {
    serde_json::from_str(source).unwrap()
}

fn context(request: &ResponsesApiRequestParams) -> RequestContext<'_> {
    RequestContext {
        request,
        provider: None,
        stream: None,
        extra_headers: None,
        metadata: None,
        service_tier: None,
        api_base: None,
    }
}

fn wire(completion: &ChatCompletionRequest) -> Value {
    serde_json::to_value(completion).unwrap()
}

fn expected_value(source: &str) -> Value {
    serde_json::from_str(source).unwrap()
}

fn mapped_function_tool() -> Value {
    expected_value(
        r#"{"type":"function","function":{"name":"lookup","description":"Find","parameters":{"type":"object","properties":{}},"strict":false}}"#,
    )
}

#[test]
fn supported_options_list_the_responses_vocabulary() {
    assert_eq!(
        SUPPORTED_RESPONSES_OPTIONS,
        [
            "input",
            "model",
            "instructions",
            "max_output_tokens",
            "metadata",
            "parallel_tool_calls",
            "previous_response_id",
            "reasoning",
            "stream",
            "temperature",
            "text",
            "tool_choice",
            "tools",
            "top_p",
            "user",
        ]
    );
}

const OPTION_SOURCES_EXPECTED: &str = r#"{
    "messages": [{"role": "user", "content": "hi"}],
    "model": "m",
    "top_p": 1,
    "user": "u",
    "temperature": 0,
    "max_tokens": 7,
    "metadata": {"k": "v"},
    "service_tier": "flex",
    "safety_identifier": "s",
    "context_management": [{"type": "compaction"}],
    "custom_llm_provider": "openai",
    "extra_headers": {"x-test": "yes"}
}"#;

#[test]
fn option_sources_map_the_request_and_kwargs() {
    let request = request(
        r#"{
            "model": "m",
            "input": "hi",
            "max_output_tokens": 7,
            "temperature": 0,
            "top_p": 1,
            "user": "u",
            "safety_identifier": "s",
            "context_management": [{"type": "compaction"}],
            "metadata": {"ignored": 1},
            "service_tier": "default"
        }"#,
    );
    let metadata = serde_json::from_str::<Json>(r#"{"k": "v"}"#).unwrap();
    let extra_headers = serde_json::from_str::<Json>(r#"{"x-test": "yes"}"#).unwrap();
    let mut harness = Harness::new(None, false);
    let mut context = context(&request);
    context.provider = Some("openai");
    context.metadata = Some(&metadata);
    context.service_tier = Some("flex");
    context.extra_headers = Some(&extra_headers);
    let completion = harness.run(&context).unwrap();
    assert_eq!(wire(&completion), expected_value(OPTION_SOURCES_EXPECTED));
}

#[test]
fn absent_and_null_values_disappear_while_false_and_zero_stay() {
    let request = request(
        r#"{
            "model": "m",
            "input": "hi",
            "max_output_tokens": null,
            "temperature": 0,
            "top_p": null,
            "user": null,
            "parallel_tool_calls": false,
            "safety_identifier": null,
            "context_management": null,
            "tools": [{"type": "function", "name": "lookup"}]
        }"#,
    );
    let mut harness = Harness::new(None, false);
    let completion = harness.run(&context(&request)).unwrap();
    assert_eq!(
        wire(&completion),
        expected_value(
            r#"{
                "messages": [{"role": "user", "content": "hi"}],
                "model": "m",
                "temperature": 0,
                "parallel_tool_calls": false,
                "tools": [{
                    "type": "function",
                    "function": {
                        "name": "lookup",
                        "description": "",
                        "parameters": {"type": "object"},
                        "strict": false
                    }
                }]
            }"#
        )
    );
}

#[rstest]
#[case::absent(None)]
#[case::null(Some("null"))]
#[case::empty(Some("[]"))]
#[case::web_search_only(Some(r#"[{"type":"web_search"}]"#))]
fn tools_without_a_chat_tool_drop_the_choice_and_parallel_flag(#[case] tools: Option<&str>) {
    let tools_fragment = tools.map_or_else(String::new, |list| format!(r#","tools":{list}"#));
    let request = request(&format!(
        r#"{{"model":"m","input":"hi","tool_choice":"required","parallel_tool_calls":true{tools_fragment}}}"#
    ));
    let mut harness = Harness::new(None, false);
    let completion = harness.run(&context(&request)).unwrap();
    let value = wire(&completion);
    assert_eq!(value.get("tool_choice"), None);
    assert_eq!(value.get("tools"), None);
    assert_eq!(value.get("parallel_tool_calls"), None);
}

#[rstest]
#[case::false_kept(false)]
#[case::true_kept(true)]
fn mapped_tools_keep_the_choice_and_parallel_flag(#[case] parallel: bool) {
    let request = request(&format!(
        r#"{{"model":"m","input":"hi","tools":[{FUNCTION_TOOL}],"tool_choice":"required","parallel_tool_calls":{parallel}}}"#
    ));
    let mut harness = Harness::new(None, false);
    let completion = harness.run(&context(&request)).unwrap();
    assert_eq!(
        wire(&completion),
        expected_value(&format!(
            r#"{{
                "messages": [{{"role": "user", "content": "hi"}}],
                "model": "m",
                "tool_choice": "required",
                "tools": [{}],
                "parallel_tool_calls": {parallel}
            }}"#,
            mapped_function_tool()
        ))
    );
}

#[test]
fn stream_true_adds_usage_options_and_updates_the_logging_sink() {
    let request = request(
        r#"{"model":"m","input":"hi","stream":false,"stream_options":{"include_obfuscation":false}}"#,
    );
    let mut harness = Harness::new(None, false);
    let mut context = context(&request);
    context.stream = Some(true);
    let completion = harness.run(&context).unwrap();
    let options = StreamOptions {
        include_usage: true,
    };
    assert_eq!(completion.stream_options, Some(options));
    assert_eq!(harness.sink.options, Some(options));
    assert_eq!(
        wire(&completion)["stream_options"],
        expected_value(r#"{"include_usage": true}"#)
    );
}

#[rstest]
#[case::explicit_false(Some(false))]
#[case::absent(None)]
fn stream_without_true_stays_and_leaves_the_sink_alone(#[case] stream: Option<bool>) {
    let request = request(
        r#"{"model":"m","input":"hi","stream":true,"stream_options":{"include_obfuscation":false}}"#,
    );
    let mut harness = Harness::new(None, false);
    let mut context = context(&request);
    context.stream = stream;
    let completion = harness.run(&context).unwrap();
    assert_eq!(completion.stream_options, None);
    assert_eq!(harness.sink.options, None);
    let value = wire(&completion);
    assert_eq!(value.get("stream_options"), None);
    assert_eq!(value.get("stream").cloned(), stream.map(Value::Bool));
}

#[test]
fn input_conversion_replays_reasoning_and_includes_instructions() {
    let request = request(
        r#"{"model":"m","instructions":"follow","input":[{"type":"reasoning","content":" x "}]}"#,
    );
    let mut harness = Harness::new(None, false);
    let completion = harness.run(&context(&request)).unwrap();
    assert_eq!(
        wire(&completion)["messages"],
        expected_value(r#"[{"role":"system","content":"follow"},{"role":"user","content":" x "}]"#)
    );
}

#[rstest]
#[case::absent(None)]
#[case::null(Some("null"))]
#[case::empty_object(Some("{}"))]
#[case::empty_string(Some(r#""""#))]
fn absent_reasoning_has_no_effort_or_summary(#[case] reasoning: Option<&str>) {
    let reasoning_fragment =
        reasoning.map_or_else(String::new, |value| format!(r#","reasoning":{value}"#));
    let request = request(&format!(
        r#"{{"model":"m","input":"hi"{reasoning_fragment}}}"#
    ));
    let mut harness = Harness::new(None, true);
    let completion = harness.run(&context(&request)).unwrap();
    assert_eq!(completion.reasoning_effort, None);
    assert_eq!(completion.reasoning_summary, None);
    assert!(harness.probe.calls().is_empty());
}

#[rstest]
#[case::string(r#""high""#, Some("high"), None)]
#[case::object_effort(r#"{"effort":"high"}"#, Some("high"), None)]
#[case::object_null_effort(r#"{"effort":null}"#, None, None)]
fn reasoning_forms_split_effort_and_summary(
    #[case] reasoning: &str,
    #[case] effort: Option<&str>,
    #[case] summary: Option<&str>,
) {
    let request = request(&format!(
        r#"{{"model":"m","input":"hi","reasoning":{reasoning}}}"#
    ));
    let mut harness = Harness::new(None, true);
    let completion = harness.run(&context(&request)).unwrap();
    assert_eq!(completion.reasoning_effort.as_deref(), effort);
    assert_eq!(completion.reasoning_summary.as_deref(), summary);
}

#[rstest]
#[case::bridging_model(true, Some("auto"))]
#[case::chat_model(false, None)]
fn summary_rides_only_a_request_that_bridges_back(
    #[case] bridged: bool,
    #[case] summary: Option<&str>,
) {
    let request =
        request(r#"{"model":"m","input":"hi","reasoning":{"effort":"high","summary":"auto"}}"#);
    let mut harness = Harness::new(None, bridged);
    let completion = harness.run(&context(&request)).unwrap();
    assert_eq!(completion.reasoning_summary.as_deref(), summary);
    assert_eq!(completion.reasoning_effort.as_deref(), Some("high"));
    assert_eq!(harness.probe.calls().len(), 1);
}

#[test]
fn probe_failure_drops_only_the_summary() {
    let request =
        request(r#"{"model":"m","input":"hi","reasoning":{"effort":"high","summary":"auto"}}"#);
    let mut harness = Harness::new(None, false);
    harness.probe = Probe::failing("probe is down");
    let completion = harness.run(&context(&request)).unwrap();
    assert_eq!(completion.reasoning_effort.as_deref(), Some("high"));
    assert_eq!(completion.reasoning_summary, None);
}

#[test]
fn probe_receives_the_retained_web_search_and_mapped_arguments() {
    let request = request(&format!(
        r#"{{
            "model": "m",
            "input": "hi",
            "tools": [
                {{"type":"web_search","search_context_size":"high","user_location":{{"type":"approximate"}}}},
                {FUNCTION_TOOL}
            ],
            "reasoning": {{"effort":"low","summary":"auto"}}
        }}"#
    ));
    let mut harness = Harness::new(Some(vec!["web_search_options".to_owned()]), true);
    let mut context = context(&request);
    context.provider = Some("openai");
    context.api_base = Some("https://api.example/v1");
    harness.run(&context).unwrap();
    assert_eq!(
        harness.parameters.calls(),
        [ParameterCall {
            model: "m".to_owned(),
            custom_llm_provider: Some("openai".to_owned()),
        }]
    );
    assert_eq!(
        harness.probe.calls(),
        [ProbeCall {
            model: "m".to_owned(),
            custom_llm_provider: "openai".to_owned(),
            web_search_options: Some(expected_value(
                r#"{"search_context_size": "high", "user_location": {"type": "approximate"}}"#
            )),
            tools: vec![mapped_function_tool()],
            reasoning_effort: Some("low".to_owned()),
            reasoning_summary: Some(Value::String("auto".to_owned())),
            api_base: Some("https://api.example/v1".to_owned()),
        }]
    );
}

#[test]
fn probe_receives_an_empty_provider_as_an_empty_string() {
    let request =
        request(r#"{"model":"m","input":"hi","reasoning":{"effort":"low","summary":"auto"}}"#);
    let mut harness = Harness::new(None, false);
    harness.run(&context(&request)).unwrap();
    assert_eq!(harness.probe.calls()[0].custom_llm_provider, "");
}

#[rstest]
#[case::supported(None, true)]
#[case::listed(Some(vec!["tools".to_owned(), "web_search_options".to_owned()]), true)]
#[case::omitted(Some(vec!["tools".to_owned()]), false)]
#[case::empty_list(Some(vec![]), false)]
fn derived_web_search_options_drop_only_when_the_capability_list_omits_them(
    #[case] supported: Option<Vec<String>>,
    #[case] retained: bool,
) {
    let request = request(r#"{"model":"m","input":"hi","tools":[{"type":"web_search"}]}"#);
    let mut harness = Harness::new(supported, false);
    let completion = harness.run(&context(&request)).unwrap();
    assert_eq!(completion.web_search_options.is_some(), retained);
}

#[test]
fn sdk_bridge_probe_reads_the_responses_mode_from_b158() {
    let responses = Catalog {
        mode: Some(Value::String("responses".to_owned())),
    };
    let no_tools: Vec<Value> = Vec::new();
    let request = BridgeRequest {
        model: "m",
        custom_llm_provider: "openai",
        web_search_options: None,
        tools: &no_tools,
        reasoning_effort: None,
        reasoning_summary: None,
        api_base: None,
        route_all_openai_to_responses: false,
    };
    let probe = SdkBridgeProbe {
        capabilities: &responses,
    };
    assert!(probe.bridges_back(&request).unwrap());
    let chat = Catalog { mode: None };
    let chat_probe = SdkBridgeProbe {
        capabilities: &chat,
    };
    assert!(!chat_probe.bridges_back(&request).unwrap());
    let route_all = BridgeRequest {
        route_all_openai_to_responses: true,
        ..request
    };
    assert!(chat_probe.bridges_back(&route_all).unwrap());
}

struct Catalog {
    mode: Option<Value>,
}

impl ResponsesBridgeCapabilities for Catalog {
    fn model_info(
        &self,
        _model: &str,
        _custom_llm_provider: &str,
    ) -> Result<Object, ModelInfoError> {
        let mut info = Object::new();
        if let Some(mode) = &self.mode {
            info.insert("mode".to_owned(), mode.clone());
        }
        Ok(info)
    }

    fn resolved_openai_api_base(&self, api_base: Option<&str>) -> String {
        api_base.unwrap_or_default().to_owned()
    }

    fn foundry_supports_responses(&self, _model: &str, _api_base: Option<&str>) -> bool {
        false
    }

    fn foundry_chat_rejects_tools(
        &self,
        _model: &str,
        _reasoning_effort: Option<&ReasoningEffort>,
    ) -> bool {
        false
    }

    fn is_gpt_5_model(&self, _model: &str) -> bool {
        false
    }

    fn is_gpt_5_4_plus_model(&self, _model: &str) -> bool {
        false
    }

    fn is_gpt_5_search_model(&self, _model: &str) -> bool {
        false
    }
}

#[rstest]
#[case::json_schema_defaults(
    r#"{"format":{"type":"json_schema"}}"#,
    Some(r#"{"type":"json_schema","json_schema":{"name":"response_schema","schema":{},"strict":false}}"#)
)]
#[case::json_schema_values(
    r#"{"format":{"type":"json_schema","name":"s","schema":{"type":"object"},"strict":true}}"#,
    Some(r#"{"type":"json_schema","json_schema":{"name":"s","schema":{"type":"object"},"strict":true}}"#)
)]
#[case::json_schema_explicit_nulls(
    r#"{"format":{"type":"json_schema","name":null,"schema":null,"strict":null}}"#,
    Some(r#"{"type":"json_schema","json_schema":{"name":null,"schema":null,"strict":null}}"#)
)]
#[case::json_object(
    r#"{"format":{"type":"json_object"}}"#,
    Some(r#"{"type":"json_object"}"#)
)]
#[case::text(r#"{"format":{"type":"text"}}"#, None)]
#[case::unknown(r#"{"format":{"type":"mystery"}}"#, None)]
#[case::format_without_type(r#"{"format":{}}"#, None)]
#[case::non_object_format(r#"{"format":"json_schema"}"#, None)]
#[case::missing_format("{}", None)]
#[case::null("null", None)]
#[case::non_object(r#""text""#, None)]
fn response_formats_follow_the_text_format(#[case] text: &str, #[case] expected: Option<&str>) {
    let request = request(&format!(r#"{{"model":"m","input":"hi","text":{text}}}"#));
    let mut harness = Harness::new(None, false);
    let completion = harness.run(&context(&request)).unwrap();
    assert_eq!(
        wire(&completion).get("response_format").cloned(),
        expected.map(expected_value)
    );
}

#[rstest]
#[case::tools(r#"{"model":"m","input":"hi","tools":5}"#, "tools")]
#[case::reasoning(r#"{"model":"m","input":"hi","reasoning":5}"#, "reasoning")]
#[case::effort(
    r#"{"model":"m","input":"hi","reasoning":{"effort":5}}"#,
    "reasoning.effort"
)]
#[case::summary(
    r#"{"model":"m","input":"hi","reasoning":{"summary":5}}"#,
    "reasoning.summary"
)]
fn wrong_types_name_their_field(#[case] source: &str, #[case] field: &str) {
    let request = request(source);
    let mut harness = Harness::new(None, true);
    let error = harness.run(&context(&request)).unwrap_err();
    assert!(matches!(error, ChatOptionsError::WrongType { field: actual } if actual == field));
}

#[test]
fn a_non_object_tool_item_is_a_tools_error() {
    let request = request(r#"{"model":"m","input":"hi","tools":["web_search"]}"#);
    let mut harness = Harness::new(None, false);
    assert!(matches!(
        harness.run(&context(&request)).unwrap_err(),
        ChatOptionsError::Tools(_)
    ));
}

#[test]
fn a_non_json_input_is_a_messages_error() {
    let request = request(r#"{"model":"m","input":5}"#);
    let mut harness = Harness::new(None, false);
    assert!(matches!(
        harness.run(&context(&request)).unwrap_err(),
        ChatOptionsError::Messages(_)
    ));
}

#[test]
fn a_null_kwarg_metadata_and_headers_do_not_reach_the_request() {
    let request = request(r#"{"model":"m","input":"hi"}"#);
    let metadata = Json::Null;
    let extra_headers = Json::Null;
    let mut harness = Harness::new(None, false);
    let mut context = context(&request);
    context.metadata = Some(&metadata);
    context.extra_headers = Some(&extra_headers);
    let completion = harness.run(&context).unwrap();
    let value = wire(&completion);
    assert_eq!(value.get("metadata"), None);
    assert_eq!(value.get("extra_headers"), None);
}

#[test]
fn an_empty_provider_is_forwarded_as_an_empty_string() {
    let request = request(r#"{"model":"m","input":"hi"}"#);
    let mut harness = Harness::new(None, false);
    let mut context = context(&request);
    context.provider = Some("");
    let completion = harness.run(&context).unwrap();
    assert_eq!(
        wire(&completion)["custom_llm_provider"],
        Value::String(String::new())
    );
}

#[test]
fn chat_completion_session_keeps_omitted_fields_omitted() {
    let session = ChatCompletionSession {
        messages: Some(vec![expected_value(r#"{"role": "user", "content": "hi"}"#)]),
        litellm_session_id: None,
    };
    assert_eq!(
        serde_json::to_value(&session).unwrap(),
        expected_value(r#"{"messages": [{"role": "user", "content": "hi"}]}"#)
    );
    assert_eq!(
        serde_json::to_value(ChatCompletionSession::default()).unwrap(),
        expected_value("{}")
    );
}
