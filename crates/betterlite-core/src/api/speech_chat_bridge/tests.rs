#![cfg(test)]

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::{HiddenParams, Json};
use betterlite_openai::chat::response::ChatResponse;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use thiserror::Error;
use tokio::runtime::{Builder, Runtime};

use super::arguments::SpeechBridgeArguments;
use super::error::{SpeechBridgeError, SpeechErrorFields, SpeechTransformError};
use super::response::SpeechBinaryResponse;
use super::{
    SpeechCompletionAnswer, SpeechCompletionCall, SpeechCompletionOutput, SpeechCompletions,
    SpeechToCompletionBridge,
};

const GEMINI_TTS_MODEL: &str = "gemini-2.5-flash-preview-tts";
const INIT: &str = r#"{"model":"gpt-4o-audio-preview","call_type":"speech","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;

#[derive(Debug, Error)]
#[error("the scripted completion failed")]
struct ScriptedFailure;

#[derive(Clone, Default)]
struct Calls {
    recorded: Arc<Mutex<Vec<IndexMap<String, Json>>>>,
}

impl Calls {
    fn record(&self, kwargs: IndexMap<String, Json>) {
        self.recorded
            .lock()
            .expect("the call recording")
            .push(kwargs);
    }

    fn snapshot(&self) -> Vec<IndexMap<String, Json>> {
        let recorded = self.recorded.lock().expect("the call recording");
        recorded.clone()
    }
}

// the completion answers the bridge's seam with, or the failure B004 would report
enum Scripted {
    Response {
        response: Box<ChatResponse>,
        hidden: HiddenParams,
    },
    Unmapped {
        type_name: &'static str,
    },
    Failure,
}

struct ScriptedCompletions {
    scripted: Scripted,
    calls: Calls,
}

impl ScriptedCompletions {
    fn answer(&self) -> Result<SpeechCompletionOutput, ScriptedFailure> {
        match &self.scripted {
            Scripted::Response { response, hidden } => Ok(SpeechCompletionOutput::Response(
                Box::new(SpeechCompletionAnswer {
                    response: response.as_ref().clone(),
                    hidden_params: hidden.clone(),
                }),
            )),
            Scripted::Unmapped { type_name } => Ok(SpeechCompletionOutput::Unmapped {
                type_name: String::from(*type_name),
            }),
            Scripted::Failure => Err(ScriptedFailure),
        }
    }
}

impl SpeechCompletions for ScriptedCompletions {
    type Error = ScriptedFailure;

    fn speech_completion(
        &self,
        call: SpeechCompletionCall<'_>,
    ) -> impl Future<Output = Result<SpeechCompletionOutput, ScriptedFailure>> + Send {
        self.calls.record(call.kwargs);
        let answer = self.answer();
        async move { answer }
    }
}

fn recording(scripted: Scripted) -> (ScriptedCompletions, Calls) {
    let calls = Calls::default();
    (
        ScriptedCompletions {
            scripted,
            calls: calls.clone(),
        },
        calls,
    )
}

struct Call {
    model: Json,
    input: Json,
    voice: Json,
    optional_params: Json,
    litellm_params: Json,
    headers: Json,
    custom_llm_provider: Json,
}

impl Call {
    fn valid() -> Self {
        Self {
            model: Json::Text(String::from("gpt-4o-audio-preview")),
            input: Json::Text(String::from("hello")),
            voice: Json::Text(String::from("alloy")),
            optional_params: empty(),
            litellm_params: empty(),
            headers: empty(),
            custom_llm_provider: Json::Text(String::from("openai")),
        }
    }

    fn arguments(&self) -> SpeechBridgeArguments<'_> {
        SpeechBridgeArguments {
            model: Some(&self.model),
            input: Some(&self.input),
            voice: Some(&self.voice),
            optional_params: Some(&self.optional_params),
            litellm_params: Some(&self.litellm_params),
            headers: Some(&self.headers),
            custom_llm_provider: Some(&self.custom_llm_provider),
        }
    }
}

fn empty() -> Json {
    Json::Object(IndexMap::new())
}

fn json_value(source: &str) -> Json {
    serde_json::from_str(source).expect("a JSON value")
}

fn projection(source: &str) -> IndexMap<String, Json> {
    serde_json::from_str(source).expect("a JSON object")
}

fn logging_state() -> LoggingState {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor");
    let mut context = CorrelationContext::default();
    LoggingState::new(
        serde_json::from_str::<LoggingInit>(INIT).expect("the logging init"),
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || String::from("trace"),
        Duration::default,
    )
    .expect("the logging state")
}

fn runtime() -> Runtime {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the test runtime")
}

fn responded(model: &str, audio: Option<&str>) -> ChatResponse {
    let audio = audio.map_or_else(String::new, |data| {
        format!(r#", "audio": {{"data": "{data}"}}"#)
    });
    let body = format!(
        r#"{{"id": "chatcmpl-1", "object": "chat.completion", "created": 1, "model": {model:?},
            "choices": [{{"index": 0, "message": {{"role": "assistant", "content": null{audio}}},
            "finish_reason": "stop"}}],
            "usage": {{"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0}}}}"#
    );
    serde_json::from_str(&body).expect("the provider response")
}

fn answered(response: ChatResponse) -> Scripted {
    Scripted::Response {
        response: Box::new(response),
        hidden: HiddenParams::new(),
    }
}

fn completed(
    scripted: Scripted,
    arguments: SpeechBridgeArguments<'_>,
) -> (SpeechBinaryResponse, Vec<IndexMap<String, Json>>) {
    let (completions, calls) = recording(scripted);
    let bridge = SpeechToCompletionBridge { completions };
    let mut logging = logging_state();
    let answer = runtime()
        .block_on(bridge.speech(arguments, Some(&mut logging)))
        .expect("the bridge answer");
    (answer, calls.snapshot())
}

fn transform_failure(
    scripted: Scripted,
    arguments: SpeechBridgeArguments<'_>,
) -> SpeechTransformError {
    let (completions, _) = recording(scripted);
    let bridge = SpeechToCompletionBridge { completions };
    let mut logging = logging_state();
    let error = runtime()
        .block_on(bridge.speech(arguments, Some(&mut logging)))
        .expect_err("a failed transformation");
    match error {
        SpeechBridgeError::Transform(error) => error,
        other => panic!("expected a transformation error, got {other}"),
    }
}

fn rejected(arguments: SpeechBridgeArguments<'_>) -> String {
    let mut logging = logging_state();
    reject(arguments, Some(&mut logging))
}

fn without_logging(arguments: SpeechBridgeArguments<'_>) -> String {
    reject(arguments, None)
}

fn reject(arguments: SpeechBridgeArguments<'_>, logging: Option<&mut LoggingState>) -> String {
    let (completions, calls) = recording(Scripted::Unmapped { type_name: "dict" });
    let bridge = SpeechToCompletionBridge { completions };
    let error = runtime()
        .block_on(bridge.speech(arguments, logging))
        .expect_err("a rejected bridge call");
    assert!(
        calls.snapshot().is_empty(),
        "validation must not reach the completion"
    );
    error.to_string()
}

fn content_type(answer: &SpeechBinaryResponse) -> Option<&str> {
    answer
        .content
        .headers
        .get("content-type")
        .map(String::as_str)
}

fn gemini_call(response_format: &str) -> Call {
    let optional_params = format!(r#"{{"response_format": "{response_format}"}}"#);
    Call {
        model: Json::Text(String::from(GEMINI_TTS_MODEL)),
        voice: Json::Text(String::from("Kore")),
        optional_params: json_value(&optional_params),
        ..Call::valid()
    }
}

#[test]
fn speech_chat_request() {
    let call = Call {
        optional_params: json_value(r#"{"temperature": 0, "response_format": "mp3", "speed": 2}"#),
        headers: json_value(r#"{"x": "a"}"#),
        ..Call::valid()
    };
    let (answer, calls) = completed(
        answered(responded("gpt-4o-audio-preview", Some("YQ=="))),
        call.arguments(),
    );
    assert_eq!(answer.content.status, 200);
    assert_eq!(answer.content.body, b"a");
    assert_eq!(answer.response_cost, None);
    assert_eq!(content_type(&answer), Some("audio/mpeg"));
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0],
        projection(
            r#"{"model": "gpt-4o-audio-preview",
                "messages": [{"role": "user", "content": "hello"}],
                "modalities": ["audio"],
                "temperature": 0,
                "audio": {"voice": "alloy", "format": "mp3"},
                "headers": {"x": "a"},
                "custom_llm_provider": "openai"}"#
        )
    );
}

#[test]
fn projection_keeps_the_litellm_order() {
    let (_, calls) = completed(
        answered(responded("m", Some("YQ=="))),
        Call::valid().arguments(),
    );
    assert_eq!(
        calls[0].keys().map(String::as_str).collect::<Vec<_>>(),
        [
            "model",
            "messages",
            "modalities",
            "audio",
            "headers",
            "custom_llm_provider"
        ]
    );
}

#[test]
fn litellm_params_win_and_nullable_options_drop() {
    let call = Call {
        optional_params: json_value(r#"{"temperature": null, "top_p": 0.5}"#),
        litellm_params: json_value(
            r#"{"model": "other-model", "messages": [{"role": "system", "content": "s"}],
                "modalities": ["text"], "audio": {"voice": "x"}, "headers": {"y": "b"},
                "custom_llm_provider": "anthropic", "litellm_logging_obj": {"a": 1}}"#,
        ),
        headers: json_value(r#"{"x": "a"}"#),
        ..Call::valid()
    };
    let (_, calls) = completed(
        answered(responded("other-model", Some("YQ=="))),
        call.arguments(),
    );
    let kwargs = &calls[0];
    assert_eq!(
        kwargs.get("model"),
        Some(&Json::Text(String::from("other-model")))
    );
    assert_eq!(
        kwargs.get("messages"),
        Some(&json_value(r#"[{"role": "system", "content": "s"}]"#))
    );
    assert_eq!(kwargs.get("modalities"), Some(&json_value(r#"["text"]"#)));
    assert_eq!(kwargs.get("audio"), Some(&json_value(r#"{"voice": "x"}"#)));
    assert_eq!(kwargs.get("headers"), Some(&json_value(r#"{"x": "a"}"#)));
    assert_eq!(
        kwargs.get("custom_llm_provider"),
        Some(&Json::Text(String::from("openai")))
    );
    assert_eq!(kwargs.get("temperature"), None);
    assert_eq!(kwargs.get("litellm_logging_obj"), None);
    assert_eq!(kwargs.get("top_p"), Some(&json_value("0.5")));
}

#[test]
fn voice_shapes_control_the_audio_param() {
    let dict_voice = Call {
        voice: json_value(r#"{"voice": "alloy"}"#),
        ..Call::valid()
    };
    let (_, dict_calls) = completed(
        answered(responded("m", Some("YQ=="))),
        dict_voice.arguments(),
    );
    assert_eq!(dict_calls[0].get("audio"), None);

    let call = Call::valid();
    let mut missing = call.arguments();
    missing.voice = None;
    let (_, missing_calls) = completed(answered(responded("m", Some("YQ=="))), missing);
    assert_eq!(missing_calls[0].get("audio"), None);

    let empty_voice = Call {
        voice: Json::Text(String::new()),
        ..Call::valid()
    };
    let (_, empty_calls) = completed(
        answered(responded("m", Some("YQ=="))),
        empty_voice.arguments(),
    );
    assert_eq!(
        empty_calls[0].get("audio"),
        Some(&json_value(r#"{"voice": ""}"#))
    );
}

#[test]
fn optional_params_audio_is_replaced() {
    let replaced = Call {
        voice: json_value(r#"{"voice": "alloy"}"#),
        optional_params: json_value(r#"{"audio": {"voice": "x", "format": "flac"}}"#),
        ..Call::valid()
    };
    let (_, replaced_calls) =
        completed(answered(responded("m", Some("YQ=="))), replaced.arguments());
    assert_eq!(replaced_calls[0].get("audio"), None);

    let overridden = Call {
        optional_params: json_value(r#"{"audio": {"voice": "x", "format": "flac"}}"#),
        ..Call::valid()
    };
    let (_, overridden_calls) = completed(
        answered(responded("m", Some("YQ=="))),
        overridden.arguments(),
    );
    assert_eq!(
        overridden_calls[0].get("audio"),
        Some(&json_value(r#"{"voice": "alloy"}"#))
    );
}

#[test]
fn gemini_pcm_and_wav() {
    let (pcm_answer, pcm_calls) = completed(
        answered(responded(GEMINI_TTS_MODEL, Some("AAA="))),
        gemini_call("pcm").arguments(),
    );
    assert_eq!(
        pcm_calls[0].get("audio"),
        Some(&json_value(r#"{"voice": "Kore", "format": "pcm16"}"#))
    );
    assert_eq!(pcm_answer.content.body, [0x00, 0x00]);
    assert_eq!(content_type(&pcm_answer), Some("audio/pcm"));

    let (wav_answer, wav_calls) = completed(
        answered(responded(GEMINI_TTS_MODEL, Some("AAA="))),
        gemini_call("wav").arguments(),
    );
    assert_eq!(
        wav_calls[0].get("audio"),
        Some(&json_value(r#"{"voice": "Kore", "format": "pcm16"}"#))
    );
    assert_eq!(content_type(&wav_answer), Some("audio/wav"));
}

#[test]
fn gemini_wav_container() {
    let (answer, _) = completed(
        answered(responded(GEMINI_TTS_MODEL, Some("AAA="))),
        gemini_call("wav").arguments(),
    );
    let body = answer.content.body;
    assert_eq!(body.len(), 46);
    assert_eq!(body.get(..4), Some(b"RIFF".as_slice()));
    assert_eq!(body.get(4..8), Some(38_u32.to_le_bytes().as_slice()));
    assert_eq!(body.get(8..12), Some(b"WAVE".as_slice()));
    assert_eq!(body.get(20..22), Some(1_u16.to_le_bytes().as_slice()));
    assert_eq!(body.get(22..24), Some(1_u16.to_le_bytes().as_slice()));
    assert_eq!(body.get(24..28), Some(24_000_u32.to_le_bytes().as_slice()));
    assert_eq!(body.get(28..32), Some(48_000_u32.to_le_bytes().as_slice()));
    assert_eq!(body.get(32..34), Some(2_u16.to_le_bytes().as_slice()));
    assert_eq!(body.get(34..36), Some(16_u16.to_le_bytes().as_slice()));
    assert_eq!(body.get(40..44), Some(2_u32.to_le_bytes().as_slice()));
}

#[test]
fn gemini_odd_pcm_is_unpadded() {
    let (answer, _) = completed(
        answered(responded(GEMINI_TTS_MODEL, Some("AAAA"))),
        gemini_call("wav").arguments(),
    );
    let body = answer.content.body;
    assert_eq!(body.len(), 47);
    assert_eq!(body.get(4..8), Some(39_u32.to_le_bytes().as_slice()));
    assert_eq!(body.get(40..44), Some(3_u32.to_le_bytes().as_slice()));
    assert_eq!(body.get(44..47), Some([0_u8, 0, 0].as_slice()));
}

#[test]
fn gemini_defaults_to_wav() {
    let (answer, calls) = completed(
        answered(responded(GEMINI_TTS_MODEL, Some("AAA="))),
        Call {
            model: Json::Text(String::from(GEMINI_TTS_MODEL)),
            ..Call::valid()
        }
        .arguments(),
    );
    assert_eq!(
        calls[0].get("audio"),
        Some(&json_value(r#"{"voice": "alloy", "format": "pcm16"}"#))
    );
    assert_eq!(content_type(&answer), Some("audio/wav"));
    let (numeric, _) = completed(
        answered(responded(GEMINI_TTS_MODEL, Some("AAA="))),
        Call {
            model: Json::Text(String::from(GEMINI_TTS_MODEL)),
            optional_params: json_value(r#"{"response_format": 7}"#),
            ..Call::valid()
        }
        .arguments(),
    );
    assert_eq!(numeric.content.body.len(), 46);
    assert_eq!(content_type(&numeric), Some("audio/wav"));
}

#[test]
fn gemini_needs_both_substrings() {
    let (plain, calls) = completed(
        answered(responded("gemini-2.0-flash", Some("YQ=="))),
        Call {
            model: Json::Text(String::from("gemini-2.0-flash")),
            optional_params: json_value(r#"{"response_format": "mp3"}"#),
            ..Call::valid()
        }
        .arguments(),
    );
    assert_eq!(
        calls[0].get("audio"),
        Some(&json_value(r#"{"voice": "alloy", "format": "mp3"}"#))
    );
    assert_eq!(plain.content.body, b"a");
    assert_eq!(content_type(&plain), Some("audio/mpeg"));
    let (tts, calls) = completed(
        answered(responded("gpt-4o-mini-tts", Some("YQ=="))),
        Call {
            model: Json::Text(String::from("gpt-4o-mini-tts")),
            optional_params: json_value(r#"{"response_format": "mp3"}"#),
            ..Call::valid()
        }
        .arguments(),
    );
    assert_eq!(
        calls[0].get("audio"),
        Some(&json_value(r#"{"voice": "alloy", "format": "mp3"}"#))
    );
    assert_eq!(tts.content.body, b"a");
    assert_eq!(content_type(&tts), Some("audio/mpeg"));
}

#[test]
fn gemini_rejects_other_formats() {
    let fields = transform_failure(
        answered(responded("m", Some("YQ=="))),
        Call {
            model: Json::Text(String::from(GEMINI_TTS_MODEL)),
            optional_params: json_value(r#"{"response_format": "mp3"}"#),
            ..Call::valid()
        }
        .arguments(),
    );
    assert_eq!(
        fields.error_fields(),
        Some(SpeechErrorFields {
            status: 400,
            error_type: None,
            param: None,
            code: None,
        })
    );
    let message = rejected(
        Call {
            model: Json::Text(String::from(GEMINI_TTS_MODEL)),
            optional_params: json_value(r#"{"response_format": "mp3"}"#),
            ..Call::valid()
        }
        .arguments(),
    );
    assert_eq!(
        message,
        "Gemini TTS only produces raw PCM16 audio, so response_format='mp3' is not supported. Supported response formats: pcm, wav."
    );
    let upper = rejected(
        Call {
            model: Json::Text(String::from("GEMINI-2.5-FLASH-PREVIEW-TTS")),
            optional_params: json_value(r#"{"response_format": "mp3"}"#),
            ..Call::valid()
        }
        .arguments(),
    );
    assert_eq!(upper, message);
}

#[test]
fn response_model_selects_conversion() {
    let (answer, calls) = completed(
        answered(responded("gpt-4o-audio-preview", Some("AAA="))),
        Call {
            model: Json::Text(String::from(GEMINI_TTS_MODEL)),
            optional_params: json_value(r#"{"response_format": "pcm"}"#),
            ..Call::valid()
        }
        .arguments(),
    );
    assert_eq!(
        calls[0].get("audio"),
        Some(&json_value(r#"{"voice": "alloy", "format": "pcm16"}"#))
    );
    assert_eq!(answer.content.body, [0x00, 0x00]);
    assert_eq!(content_type(&answer), Some("audio/mpeg"));
}

#[test]
fn speech_bridge_failures() {
    let call = Call::valid();
    let mut missing_model = call.arguments();
    missing_model.model = None;
    assert_eq!(rejected(missing_model), "model is required");
    let mut missing_provider = call.arguments();
    missing_provider.custom_llm_provider = None;
    assert_eq!(
        rejected(missing_provider),
        "custom_llm_provider is required"
    );
    let mut missing_input = call.arguments();
    missing_input.input = None;
    assert_eq!(rejected(missing_input), "input is required");
    let mut missing_options = call.arguments();
    missing_options.optional_params = None;
    assert_eq!(rejected(missing_options), "optional_params is required");
    let mut missing_params = call.arguments();
    missing_params.litellm_params = None;
    assert_eq!(rejected(missing_params), "litellm_params is required");
    let mut missing_headers = call.arguments();
    missing_headers.headers = None;
    assert_eq!(rejected(missing_headers), "headers is required");
    assert_eq!(without_logging(call.arguments()), "logging_obj is required");
}

#[test]
fn rejects_wrong_argument_types() {
    let call = Call::valid();
    let wrong_text = Json::Bool(true);
    let wrong_map = Json::Text(String::from("not a map"));
    let mut wrong_model = call.arguments();
    wrong_model.model = Some(&wrong_text);
    assert_eq!(rejected(wrong_model), "model is required");
    let mut wrong_provider = call.arguments();
    wrong_provider.custom_llm_provider = Some(&wrong_text);
    assert_eq!(rejected(wrong_provider), "custom_llm_provider is required");
    let mut wrong_input = call.arguments();
    wrong_input.input = Some(&wrong_text);
    assert_eq!(rejected(wrong_input), "input is required");
    let mut wrong_options = call.arguments();
    wrong_options.optional_params = Some(&wrong_map);
    assert_eq!(rejected(wrong_options), "optional_params is required");
    let mut wrong_params = call.arguments();
    wrong_params.litellm_params = Some(&wrong_map);
    assert_eq!(rejected(wrong_params), "litellm_params is required");
    let mut wrong_headers = call.arguments();
    wrong_headers.headers = Some(&wrong_map);
    assert_eq!(rejected(wrong_headers), "headers is required");
}

#[test]
fn empty_arguments_pass() {
    let call = Call {
        model: Json::Text(String::new()),
        input: Json::Text(String::new()),
        custom_llm_provider: Json::Text(String::new()),
        ..Call::valid()
    };
    let (answer, calls) = completed(answered(responded("m", Some("YQ=="))), call.arguments());
    assert_eq!(answer.content.body, b"a");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].get("model"), Some(&Json::Text(String::new())));
}

#[test]
fn missing_audio_fails() {
    let error = transform_failure(
        answered(responded("gpt-4o-audio-preview", None)),
        Call::valid().arguments(),
    );
    assert!(matches!(error, SpeechTransformError::MissingAudio));
    assert_eq!(error.to_string(), "No audio part found in the response");
}

#[test]
fn empty_choices_fail() {
    let mut response = responded("gpt-4o-audio-preview", Some("YQ=="));
    response.choices.clear();
    let error = transform_failure(answered(response), Call::valid().arguments());
    assert!(matches!(error, SpeechTransformError::NoChoices));
    assert_eq!(error.to_string(), "list index out of range");
}

#[test]
fn response_audio_is_tolerantly_decoded() {
    let (answer, _) = completed(
        answered(responded("m", Some("Y Q==!"))),
        Call::valid().arguments(),
    );
    assert_eq!(answer.content.body, b"a");
}

#[test]
fn invalid_base64_fails() {
    let error = transform_failure(
        answered(responded("m", Some("QQ"))),
        Call::valid().arguments(),
    );
    assert!(matches!(error, SpeechTransformError::Audio(_)));
    assert_eq!(error.to_string(), "Incorrect padding");
}

#[test]
fn unmapped_completion_output_fails() {
    let error = transform_failure(
        Scripted::Unmapped { type_name: "dict" },
        Call::valid().arguments(),
    );
    assert_eq!(
        error.to_string(),
        "Unmapped response type. Got type: <class 'dict'>"
    );
}

fn retained_cost(hidden: HiddenParams) -> Option<Decimal> {
    let (answer, _) = completed(
        Scripted::Response {
            response: Box::new(responded("m", Some("YQ=="))),
            hidden,
        },
        Call::valid().arguments(),
    );
    answer.response_cost
}

#[test]
fn response_cost_keeps_only_python_floats() {
    assert_eq!(
        retained_cost(projection(r#"{"response_cost": 2.0}"#)),
        Some(Decimal::from(2))
    );
    assert_eq!(
        retained_cost(projection(r#"{"response_cost": 2.5}"#)),
        Some(Decimal::new(25, 1))
    );
    assert_eq!(retained_cost(projection(r#"{"response_cost": 2}"#)), None);
    assert_eq!(
        retained_cost(projection(r#"{"response_cost": true}"#)),
        None
    );
    assert_eq!(retained_cost(projection(r#"{"response_cost": "2"}"#)), None);
    assert_eq!(retained_cost(HiddenParams::new()), None);
}

#[test]
fn completion_failure_surfaces() {
    let (completions, _) = recording(Scripted::Failure);
    let bridge = SpeechToCompletionBridge { completions };
    let mut logging = logging_state();
    let error = runtime()
        .block_on(bridge.speech(Call::valid().arguments(), Some(&mut logging)))
        .expect_err("a failed completion");
    assert!(matches!(error, SpeechBridgeError::Completion { .. }));
    assert_eq!(error.to_string(), "the scripted completion failed");
}
