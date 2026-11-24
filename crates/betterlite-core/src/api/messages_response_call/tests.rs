#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_anthropic::stream_state::state::IdSource;
use betterlite_config::loader::{ConfigError, ConfigLoader};
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_models::full_model_metadata::record::ModelInfo;
use betterlite_models::model_metadata_resolution::names::ModelCostMap;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use rstest::rstest;
use uuid::Uuid;

use super::error::MessagesResponseCallError;
use super::records::{
    CallSettings, MessagesResponseCallAnswer, MessagesResponseCallInput,
    MessagesResponseCallResult, ResponsesAnswer,
};
use super::seams::{MessagesResponseCallSeams, ResponsesCreatePort, ResponsesIncludeSupport};
use super::service::messages_response_call;
use crate::api::api_boundaries::ApiLimits;
use crate::api::messages_reasoning_route::seams::{
    ModelMetadata, ProviderLookup, ProviderResolution, ResolvedTarget,
};
use crate::api::messages_response_stream::source::{ResponsesEvent, ResponsesEventSource};
use crate::api::messages_response_stream::{MessagesResponseStream, MessagesResponseStreamResult};
use crate::api::responses_create::call::ResponsesCreateCall;
use crate::api::responses_create::error::CreateFailure;
use crate::api::responses_create::execute::mock_responses_api_response;
use crate::api::responses_create_http::service::LogContext;
use crate::api::responses_stream_errors::failure::FailureCause;

const MESSAGE_ID: &str = "msg_00000000-0000-0000-0000-000000000007";
const MOCK_ID: &str = "resp_67ccd2bed1ec8190b14f964abc0542670bb6a6b452d3795b";
const LISTED_REQUEST: &str = r#"{"model":"openai/gpt-4o","messages":[{"role":"user","content":"hi"}],"max_tokens":128,"system":"be helpful","temperature":0.2,"top_p":0.9,"metadata":{"user_id":"user_a"}}"#;

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the test JSON parses")
}

fn value(text: &str) -> JsonValue {
    serde_json::from_str(text).expect("the test value parses")
}

fn cost() -> ModelCostMap {
    ModelCostMap::parse(b"{}").expect("the empty model map parses")
}

fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the test redactor builds")
}

fn call_input(request: &str, forwarded: &str, stream: bool) -> MessagesResponseCallInput {
    MessagesResponseCallInput {
        request: serde_json::from_str(request).expect("the request fixture parses"),
        stream,
        settings: CallSettings::default(),
        forwarded: serde_json::from_str(forwarded).expect("the forwarded fixture parses"),
    }
}

struct ScriptedInclude {
    supported: bool,
    calls: Mutex<Vec<(String, String)>>,
}

impl ResponsesIncludeSupport for ScriptedInclude {
    fn supports_include(&self, provider: &str, model: &str) -> bool {
        self.calls
            .lock()
            .expect("the include lock is free")
            .push((provider.to_owned(), model.to_owned()));
        self.supported
    }
}

struct ScriptedResolution {
    provider: Option<String>,
    lookups: Mutex<Vec<Option<String>>>,
}

impl ProviderResolution for ScriptedResolution {
    fn resolve(&self, lookup: &ProviderLookup<'_>) -> Option<ResolvedTarget> {
        self.lookups
            .lock()
            .expect("the resolution lock is free")
            .push(lookup.custom_llm_provider.map(str::to_owned));
        self.provider.as_ref().map(|provider| ResolvedTarget {
            model: lookup.model.unwrap_or_default().to_owned(),
            provider: provider.clone(),
            api_base: None,
        })
    }
}

struct ScriptedMetadata {
    info: Option<IndexMap<String, Json>>,
    cost: ModelCostMap,
    lookups: Mutex<Vec<(Option<String>, Option<String>)>>,
}

impl ModelMetadata for ScriptedMetadata {
    fn model_info(
        &self,
        model: Option<&str>,
        custom_llm_provider: Option<&str>,
    ) -> Option<ModelInfo> {
        self.lookups
            .lock()
            .expect("the metadata lock is free")
            .push((
                model.map(str::to_owned),
                custom_llm_provider.map(str::to_owned),
            ));
        self.info.clone().map(|fields| ModelInfo { fields })
    }

    fn model_cost(&self) -> &ModelCostMap {
        &self.cost
    }
}

enum AnswerPlan {
    Response(ResponsesApiResponse),
    Stream(Vec<ResponsesEvent>),
    Other(&'static str),
    Failure(CreateFailure),
}

struct ScriptedPort {
    answer: Mutex<Option<AnswerPlan>>,
    calls: Mutex<Vec<ResponsesCreateCall>>,
}

struct ScriptedEvents {
    events: VecDeque<ResponsesEvent>,
}

#[async_trait]
impl ResponsesEventSource for ScriptedEvents {
    async fn next_event(&mut self) -> Result<Option<ResponsesEvent>, FailureCause> {
        Ok(self.events.pop_front())
    }
}

#[async_trait]
impl ResponsesCreatePort for ScriptedPort {
    type Stream = ScriptedEvents;

    async fn create(
        &self,
        call: &ResponsesCreateCall,
        _log: &mut LogContext<'_, '_>,
    ) -> Result<ResponsesAnswer<Self::Stream>, CreateFailure> {
        self.calls
            .lock()
            .expect("the port lock is free")
            .push(call.clone());
        let plan = self.answer.lock().expect("the answer lock is free").take();
        match plan {
            Some(AnswerPlan::Response(response)) => {
                Ok(ResponsesAnswer::Response(Box::new(response)))
            }
            Some(AnswerPlan::Stream(events)) => Ok(ResponsesAnswer::Stream(ScriptedEvents {
                events: VecDeque::from(events),
            })),
            Some(AnswerPlan::Other(kind)) => Ok(ResponsesAnswer::Other {
                kind: kind.to_owned(),
            }),
            Some(AnswerPlan::Failure(failure)) => Err(failure),
            None => Err(unscripted()),
        }
    }
}

fn unscripted() -> CreateFailure {
    CreateFailure {
        class: "APIError",
        status: 500,
        kind: None,
        param: None,
        code: None,
        message: String::from("the scripted port ran twice"),
    }
}

struct FixedIds;

impl IdSource for FixedIds {
    fn next_id(&self) -> Uuid {
        Uuid::from_u128(7)
    }
}

struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

struct Fixture {
    include: ScriptedInclude,
    resolution: ScriptedResolution,
    metadata: ScriptedMetadata,
    port: ScriptedPort,
    redactor: Redactor,
    sink: Sink,
    now: DateTime<FixedOffset>,
}

fn fixture_with(
    supported: bool,
    provider: Option<&str>,
    info: Option<&str>,
    answer: AnswerPlan,
) -> Fixture {
    Fixture {
        include: ScriptedInclude {
            supported,
            calls: Mutex::new(Vec::new()),
        },
        resolution: ScriptedResolution {
            provider: provider.map(str::to_owned),
            lookups: Mutex::new(Vec::new()),
        },
        metadata: ScriptedMetadata {
            info: info.map(|text| serde_json::from_str(text).expect("the model info parses")),
            cost: cost(),
            lookups: Mutex::new(Vec::new()),
        },
        port: ScriptedPort {
            answer: Mutex::new(Some(answer)),
            calls: Mutex::new(Vec::new()),
        },
        redactor: redactor(),
        sink: Sink,
        now: DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00")
            .expect("the fixture clock parses"),
    }
}

impl Fixture {
    fn environment(&self) -> PrePostEnv<'_> {
        PrePostEnv {
            debug: DebugSettings::default(),
            globals: GlobalFlags::default(),
            decision: Decision {
                content: false,
                key_info: false,
            },
            redactor: &self.redactor,
            log: &self.sink,
            input_callbacks: &[],
            sentry: None,
            logger_fn: None,
            now: self.now,
        }
    }

    fn state(&self) -> LoggingState {
        let init: LoggingInit = serde_json::from_str(
            r#"{"model":"gpt-4o","call_type":"responses","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1"}"#,
        )
        .expect("the logging fixture is valid");
        LoggingState::new(
            init,
            CorrelationSetup {
                context: &mut CorrelationContext::default(),
                settings: CorrelationSettings::default(),
            },
            &self.redactor,
            || String::from("generated"),
            Duration::default(),
        )
        .expect("the logging state builds")
    }

    fn seams(&self) -> MessagesResponseCallSeams<'_> {
        MessagesResponseCallSeams {
            include: &self.include,
            provider: &self.resolution,
            metadata: &self.metadata,
            redactor: None,
            limits: ApiLimits::default(),
        }
    }

    fn seen_calls(&self) -> Vec<ResponsesCreateCall> {
        self.port
            .calls
            .lock()
            .expect("the port lock is free")
            .clone()
    }

    fn include_calls(&self) -> Vec<(String, String)> {
        self.include
            .calls
            .lock()
            .expect("the include lock is free")
            .clone()
    }

    fn resolution_calls(&self) -> Vec<Option<String>> {
        self.resolution
            .lookups
            .lock()
            .expect("the resolution lock is free")
            .clone()
    }
}

async fn run(
    fixture: &Fixture,
    input: &MessagesResponseCallInput,
) -> (
    Result<MessagesResponseCallResult<ScriptedEvents>, MessagesResponseCallError>,
    LoggingState,
) {
    let env = fixture.environment();
    let mut state = fixture.state();
    let seams = fixture.seams();
    let result = {
        let mut log = LogContext {
            state: &mut state,
            env: &env,
        };
        messages_response_call(input, &seams, &fixture.port, &FixedIds, &mut log).await
    };
    (result, state)
}

fn native() -> AnswerPlan {
    AnswerPlan::Response(mock_responses_api_response(Some("hi")).expect("the mock response builds"))
}

#[rstest]
#[case::forwarded_kwargs(
    r#"{"custom_llm_provider":"openai","api_base":"https://api.example.com","prompt_cache_key":"cache-key","include":["reasoning.encrypted_content","extra.includable"]}"#,
    true,
    r#"["reasoning.encrypted_content","extra.includable"]"#,
    "cache-key",
    "https://api.example.com"
)]
#[case::include_without_capability(r#"{"custom_llm_provider":"openai"}"#, false, "", "", "")]
#[case::explicit_prompt_cache_key_wins(
    r#"{"custom_llm_provider":"openai","api_base":"https://api.example.com","prompt_cache_key":"explicit"}"#,
    true,
    r#"["reasoning.encrypted_content"]"#,
    "explicit",
    "https://api.example.com"
)]
#[tokio::test]
async fn messages_responses_request(
    #[case] forwarded: &str,
    #[case] supported: bool,
    #[case] expected_include: &str,
    #[case] expected_key: &str,
    #[case] expected_base: &str,
) {
    let fixture = fixture_with(supported, Some("openai"), None, native());
    let input = call_input(LISTED_REQUEST, forwarded, false);
    let (result, state) = run(&fixture, &input).await;
    let call_result = result.expect("the call succeeds");

    assert_eq!(call_result.call.model, "openai/gpt-4o");
    assert_eq!(call_result.call.max_output_tokens, Some(128));
    assert_eq!(call_result.call.instructions.as_deref(), Some("be helpful"));
    assert_eq!(call_result.call.user.as_deref(), Some("user_a"));
    assert_eq!(call_result.call.temperature, Some(0.2));
    assert_eq!(call_result.call.top_p, Some(0.9));
    assert_eq!(call_result.call.stream, None);
    assert_eq!(
        call_result.call.custom_llm_provider.as_deref(),
        Some("openai")
    );
    let base = if expected_base.is_empty() {
        None
    } else {
        Some(expected_base)
    };
    assert_eq!(call_result.call.api_base.as_deref(), base);
    let include = if expected_include.is_empty() {
        None
    } else {
        Some(json(expected_include))
    };
    assert_eq!(call_result.call.include, include);
    let expected_cache_key = if expected_key.is_empty() {
        None
    } else {
        Some(value(&format!("\"{expected_key}\"")))
    };
    assert_eq!(
        call_result.call.kwargs.get("prompt_cache_key"),
        expected_cache_key.as_ref()
    );
    assert_eq!(state.call_type, "anthropic_messages");
    assert_eq!(
        fixture.include_calls(),
        vec![(String::from("openai"), String::from("gpt-4o"))]
    );
    assert!(
        fixture.resolution_calls().is_empty(),
        "a hint skips resolution"
    );
    let seen = fixture.seen_calls();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen.first().map(|call| call.model.as_str()),
        Some("openai/gpt-4o")
    );
    let MessagesResponseCallAnswer::Native(message) = &call_result.answer else {
        panic!("a native response translates");
    };
    assert_eq!(message.id, MOCK_ID);
    let rendered = serde_json::to_string(message).expect("the message serializes");
    assert!(rendered.contains("\"hi\""), "the text reaches the message");
}

#[rstest]
#[case::hinted("openai", "gpt-4o", "openai", "gpt-4o")]
#[case::hinted_with_prefix("openai", "openai/gpt-4o", "openai", "gpt-4o")]
#[case::inferred("", "gpt-4o", "openai", "gpt-4o")]
#[case::prefixed_inferred("", "perplexity/sonar", "perplexity", "sonar")]
#[tokio::test]
async fn encrypted_task_required(
    #[case] hint: &str,
    #[case] model: &str,
    #[case] resolved: &str,
    #[case] local: &str,
) {
    let request = format!(r#"{{"model":"{model}","messages":[],"max_tokens":8}}"#);
    let forwarded = if hint.is_empty() {
        String::from("{}")
    } else {
        format!(r#"{{"custom_llm_provider":"{hint}"}}"#)
    };
    let fixture = fixture_with(true, Some(resolved), None, native());
    let input = call_input(&request, &forwarded, false);
    let (result, _) = run(&fixture, &input).await;
    let call = result.expect("the call succeeds").call;
    assert_eq!(
        call.include,
        Some(json(r#"["reasoning.encrypted_content"]"#)),
        "a provider whose Responses config supports include gets the encrypted-reasoning task"
    );
    assert_eq!(
        fixture.include_calls(),
        vec![(resolved.to_owned(), local.to_owned())],
        "the capability is asked with the provider's own model id"
    );
    let resolutions = fixture.resolution_calls();
    if hint.is_empty() {
        assert_eq!(resolutions, vec![None]);
    } else {
        assert!(resolutions.is_empty(), "a named provider is used as given");
    }
}

#[tokio::test]
async fn encrypted_task_required_omits_an_unsupported_include() {
    let fixture = fixture_with(false, Some("perplexity"), None, native());
    let request = r#"{"model":"sonar","messages":[],"max_tokens":8}"#;
    let input = call_input(request, r#"{"custom_llm_provider":"perplexity"}"#, false);
    let (result, _) = run(&fixture, &input).await;
    let call = result.expect("the call succeeds").call;
    assert_eq!(call.include, None);
    assert_eq!(
        fixture.include_calls(),
        vec![(String::from("perplexity"), String::from("sonar"))]
    );
}

#[tokio::test]
async fn encrypted_task_required_refuses_an_unresolvable_provider() {
    let fixture = fixture_with(true, None, None, native());
    let request = r#"{"model":"nowhere/model","messages":[],"max_tokens":8}"#;
    let input = call_input(request, "{}", false);
    let (result, _) = run(&fixture, &input).await;
    let error = result.expect_err("an unresolvable provider fails before the call");
    let failure = error.normalize("nowhere/model", "openai", &fixture.redactor);
    assert_eq!(failure.status, 400);
    assert_eq!(failure.class, "BadRequestError");
    assert!(fixture.seen_calls().is_empty());
}

#[rstest]
#[case::floor("max", None, "high")]
#[case::accepted(
    "max",
    Some(r#"{"supports_reasoning":true,"reasoning_effort_levels":["max","high"]}"#),
    "max"
)]
#[case::unchained("low", None, "low")]
#[tokio::test]
async fn normalizes_reasoning_effort(
    #[case] requested: &str,
    #[case] info: Option<&str>,
    #[case] expected: &str,
) {
    let request = format!(
        r#"{{"model":"gpt-4o","messages":[],"max_tokens":64,"thinking":{{"type":"adaptive"}},"output_config":{{"effort":"{requested}"}}}}"#
    );
    let fixture = fixture_with(true, Some("openai"), info, native());
    let input = call_input(&request, r#"{"custom_llm_provider":"openai"}"#, false);
    let (result, _) = run(&fixture, &input).await;
    let call_result = result.expect("the call succeeds");
    let reasoning = call_result
        .call
        .reasoning
        .expect("a thinking request carries effort");
    let effort = reasoning
        .as_object()
        .and_then(|fields| fields.get("effort"))
        .and_then(Json::as_str);
    assert_eq!(
        effort,
        Some(expected),
        "LiteLLM lowers a tier the deployment does not accept"
    );
}

#[rstest]
#[case::other_result("other", false)]
#[case::stream_for_native("stream", false)]
#[case::response_for_stream("response", true)]
#[tokio::test]
async fn native_responses_error(#[case] plan: &str, #[case] stream: bool) {
    let answer = match plan {
        "other" => AnswerPlan::Other("a dict"),
        "stream" => AnswerPlan::Stream(Vec::new()),
        "response" => native(),
        _ => panic!("the case names a scripted answer"),
    };
    let fixture = fixture_with(true, Some("openai"), None, answer);
    let input = call_input(
        LISTED_REQUEST,
        r#"{"custom_llm_provider":"openai"}"#,
        stream,
    );
    let (result, _) = run(&fixture, &input).await;
    let error = result.expect_err("a result the handler cannot use fails");
    let failure = error.normalize("openai/gpt-4o", "openai", &fixture.redactor);
    assert_eq!(failure.status, 500);
    assert_eq!(failure.class, "APIConnectionError");
    assert_eq!(failure.kind, None);
    assert_eq!(failure.param, None);
    assert_eq!(failure.code, None);
}

#[tokio::test]
async fn native_responses_error_keeps_a_normalized_failure() {
    let failure = CreateFailure {
        class: "RateLimitError",
        status: 429,
        kind: Some(String::from("rate_limit_error")),
        param: None,
        code: Some(String::from("429")),
        message: String::from("slow down"),
    };
    let fixture = fixture_with(true, Some("openai"), None, AnswerPlan::Failure(failure));
    let input = call_input(LISTED_REQUEST, r#"{"custom_llm_provider":"openai"}"#, false);
    let (result, _) = run(&fixture, &input).await;
    let error = result.expect_err("a failed create fails the call");
    let normalized = error.normalize("openai/gpt-4o", "openai", &fixture.redactor);
    assert_eq!(normalized.status, 429);
    assert_eq!(normalized.class, "RateLimitError");
    assert_eq!(normalized.code.as_deref(), Some("429"));
}

async fn first_chunk<S>(stream: &mut MessagesResponseStream<S>) -> Json
where
    S: ResponsesEventSource,
{
    match stream
        .next_chunk()
        .await
        .expect("the stream yields its message_start")
    {
        MessagesResponseStreamResult::Chunk(chunk) => chunk,
        MessagesResponseStreamResult::End => panic!("the stream ended before message_start"),
    }
}

#[tokio::test]
async fn stream_responses_adapter() {
    let fixture = fixture_with(true, Some("openai"), None, AnswerPlan::Stream(Vec::new()));
    let input = call_input(LISTED_REQUEST, r#"{"custom_llm_provider":"openai"}"#, true);
    let (result, state) = run(&fixture, &input).await;
    let call_result = result.expect("the streaming call succeeds");
    assert_eq!(call_result.call.stream, Some(true));
    let MessagesResponseCallAnswer::Stream(mut stream) = call_result.answer else {
        panic!("a streaming call returns the wrapper");
    };
    let chunk = first_chunk(&mut stream).await;
    let fields = chunk.as_object().expect("the chunk is an object");
    assert_eq!(
        fields.get("type").and_then(Json::as_str),
        Some("message_start")
    );
    let message = fields
        .get("message")
        .and_then(Json::as_object)
        .expect("message_start carries the message");
    assert_eq!(message.get("id").and_then(Json::as_str), Some(MESSAGE_ID));
    assert_eq!(
        message.get("model").and_then(Json::as_str),
        Some("gpt-4o"),
        "the wrapper reports the provider's own model id"
    );
    assert_eq!(
        state.streamed_anthropic_message_id.as_deref(),
        Some(MESSAGE_ID)
    );
}

const ABSENT_MAX_TOKENS: &str = r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#;
const MAX_TOKENS_VARIABLE: &str = "DEFAULT_MAX_TOKENS";

fn loaded_default(environment: &[(&str, &str)]) -> Result<i64, ConfigError> {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load("")
        .map(|config| config.litellm_settings.default_max_tokens)
}

#[rstest]
#[case::unset_defaults(&[], Some(4096))]
#[case::explicit_override(&[(MAX_TOKENS_VARIABLE, "8192")], Some(8192))]
#[case::surrounding_space(&[(MAX_TOKENS_VARIABLE, " 123 ")], Some(123))]
#[case::empty_counts_as_unset(&[(MAX_TOKENS_VARIABLE, "")], Some(4096))]
#[case::invalid_value(&[(MAX_TOKENS_VARIABLE, "4k")], None)]
#[tokio::test]
async fn loads_settings(#[case] environment: &[(&str, &str)], #[case] expected: Option<i64>) {
    let loaded = loaded_default(environment);
    let Some(tokens) = expected else {
        let error = loaded.expect_err("a value that is not a number fails the load");
        assert!(error.to_string().contains(MAX_TOKENS_VARIABLE));
        return;
    };
    let tokens_from_config = loaded.expect("the variable loads");
    assert_eq!(tokens_from_config, tokens);
    let settings = CallSettings {
        default_max_tokens: tokens_from_config,
        ..CallSettings::default()
    };

    let absent_fixture = fixture_with(true, Some("openai"), None, native());
    let mut absent = call_input(ABSENT_MAX_TOKENS, "{}", false);
    absent.settings = settings.clone();
    let (absent_result, _) = run(&absent_fixture, &absent).await;
    assert_eq!(
        absent_result
            .expect("the call succeeds")
            .call
            .max_output_tokens,
        Some(tokens_from_config),
        "an absent max_tokens takes the loaded default"
    );

    let explicit_fixture = fixture_with(true, Some("openai"), None, native());
    let mut explicit = call_input(LISTED_REQUEST, "{}", false);
    explicit.settings = settings;
    let (explicit_result, _) = run(&explicit_fixture, &explicit).await;
    assert_eq!(
        explicit_result
            .expect("the call succeeds")
            .call
            .max_output_tokens,
        Some(128),
        "a request max_tokens wins over the default"
    );
}
