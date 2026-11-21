use betterlite_context::media_config_selection::text_to_speech::TextToSpeechConfigTag;
use betterlite_models::decisions::Json;
use betterlite_openai::api::speech_provider_contract::SpeechVoice;
use serde_json::Value;

use super::super::call::SpeechCall;
use super::super::operations::SpeechOperationKind;
use super::support::*;

fn vertex_harness(configs: Configs, operations: ScriptedOperations) -> Harness<RecordingTransport> {
    Harness::new(
        loaded_settings(SOURCE, &[]),
        RecordingTransport::ok("audio/wav", &[]),
        &[],
    )
    .configs(configs)
    .operations(operations)
}

#[test]
fn vertex_dispatch_passes_the_call_headers_and_params() {
    let harness = vertex_harness(
        Configs::lyria(ScriptedConfig::keeping("{}")),
        ScriptedOperations::ok(b"audio"),
    );
    let native = SpeechCall {
        voice: dict_voice(r#"{"language_code": "en-US"}"#),
        headers: Some(object(r#"{"x-call": "yes"}"#)),
        kwargs: object(
            r#"{"extra_headers": {"x-kw": "no"}, "vertex_project": "proj",
                "vertex_location": "us-central1",
                "vertex_credentials": {"type": "service_account"}}"#,
        ),
        ..base_call("vertex_ai/text-to-speech")
    };
    harness.run_ok(&native);
    let executed = harness.operations.take();
    assert_eq!(executed.len(), 1);
    let operation = &executed[0];
    assert_eq!(operation.kind, SpeechOperationKind::Dispatch);
    assert_eq!(operation.provider, "vertex_ai");
    assert_eq!(
        operation.config_tag,
        Some(TextToSpeechConfigTag::VertexAILyria)
    );
    assert_eq!(operation.api_key, None);
    assert_eq!(
        operation.extra_headers,
        Some(object(r#"{"x-call": "yes"}"#))
    );
    assert_eq!(operation.voice, dict_voice(r#"{"language_code": "en-US"}"#));
    assert_eq!(
        operation.litellm_params.get("vertex_project"),
        Some(&Value::String(String::from("proj")))
    );
    assert_eq!(
        operation.litellm_params.get("vertex_location"),
        Some(&Value::String(String::from("us-central1")))
    );
    assert_eq!(
        operation.litellm_params.get("vertex_credentials"),
        Some(&serde_json::json!({"type": "service_account"}))
    );
    let plain_harness = vertex_harness(
        Configs::scripted(ScriptedConfig::keeping("{}")),
        ScriptedOperations::ok(b"audio"),
    );
    let plain = SpeechCall {
        voice: named_voice("en-US-Studio-O"),
        ..base_call("vertex_ai/text-to-speech")
    };
    plain_harness.run_ok(&plain);
    let plain_executed = plain_harness.operations.take();
    assert_eq!(
        plain_executed[0].config_tag,
        Some(TextToSpeechConfigTag::VertexAI)
    );
    assert_eq!(
        plain_executed[0].litellm_params.get("vertex_project"),
        Some(&Value::Null)
    );
    assert_eq!(plain_executed[0].extra_headers, None);
}

#[test]
fn vertex_ai_beta_takes_the_fallback_config_after_the_mapping() {
    let harness = vertex_harness(
        Configs::scripted(ScriptedConfig::keeping("{}")),
        ScriptedOperations::ok(b"audio"),
    );
    let native = SpeechCall {
        voice: named_voice("en-US-Studio-O"),
        ..base_call("vertex_ai_beta/text-to-speech")
    };
    harness.run_ok(&native);
    let executed = harness.operations.take();
    assert_eq!(executed.len(), 1);
    assert_eq!(executed[0].provider, "vertex_ai_beta");
    assert_eq!(
        executed[0].config_tag,
        Some(TextToSpeechConfigTag::VertexAI)
    );
    assert!(executed[0].config_present);
    let scripted = harness.configs.mapping().expect("the scripted config");
    assert!(scripted.calls().is_empty());

    let lyria_harness = vertex_harness(
        Configs::lyria(ScriptedConfig::keeping("{}")),
        ScriptedOperations::ok(b"audio"),
    );
    lyria_harness.run_ok(&native);
    let lyria_executed = lyria_harness.operations.take();
    assert_eq!(
        lyria_executed[0].config_tag,
        Some(TextToSpeechConfigTag::VertexAILyria)
    );
    assert!(lyria_executed[0].config_present);
}

#[test]
fn a_vertex_gemini_model_uses_the_completion_bridge() {
    let completions =
        ScriptedCompletions::answered(responded("gemini-2.5-preview-tts", Some("AAA=")));
    let harness = vertex_harness(Configs::absent(), ScriptedOperations::ok(b"unused"))
        .completions(completions);
    let native = SpeechCall {
        voice: named_voice("Kore"),
        headers: Some(object(r#"{"x-call": "yes"}"#)),
        ..base_call("vertex_ai/gemini-2.5-preview-tts")
    };
    let content = harness.run_ok(&native);
    assert_eq!(content.status, 200);
    assert_eq!(
        content.headers.get("content-type").map(String::as_str),
        Some("audio/wav")
    );
    assert_eq!(content.body.len(), 46);
    assert_eq!(content.body.get(..4), Some(b"RIFF".as_slice()));
    let calls = harness.completions.calls();
    assert_eq!(calls.len(), 1);
    let kwargs = &calls[0];
    assert_eq!(
        kwargs.get("model"),
        Some(&Json::Text(String::from("gemini-2.5-preview-tts")))
    );
    assert_eq!(
        kwargs.get("messages"),
        Some(&json(r#"[{"role": "user", "content": "hello"}]"#))
    );
    assert_eq!(
        kwargs.get("audio"),
        Some(&json(r#"{"voice": "Kore", "format": "pcm16"}"#))
    );
    assert_eq!(kwargs.get("headers"), Some(&json(r#"{"x-call": "yes"}"#)));
    assert_eq!(
        kwargs.get("custom_llm_provider"),
        Some(&Json::Text(String::from("vertex_ai")))
    );
    assert!(harness.operations.take().is_empty());
}

#[test]
fn a_gemini_provider_uses_the_completion_bridge_with_empty_headers() {
    let completions =
        ScriptedCompletions::answered(responded("gemini-2.5-flash-preview-tts", Some("AAA=")));
    let harness = vertex_harness(Configs::absent(), ScriptedOperations::ok(b"unused"))
        .completions(completions);
    let native = SpeechCall {
        voice: named_voice("Kore"),
        ..base_call("gemini/gemini-2.5-flash-preview-tts")
    };
    let content = harness.run_ok(&native);
    assert_eq!(content.body.len(), 46);
    let calls = harness.completions.calls();
    assert_eq!(
        calls[0].get("headers"),
        Some(&Json::Object(indexmap::IndexMap::new()))
    );
    assert_eq!(
        calls[0].get("model"),
        Some(&Json::Text(String::from("gemini-2.5-flash-preview-tts")))
    );
    assert_eq!(
        calls[0].get("custom_llm_provider"),
        Some(&Json::Text(String::from("gemini")))
    );
}

#[test]
fn the_vertex_gemini_check_is_case_sensitive() {
    let harness = vertex_harness(Configs::absent(), ScriptedOperations::ok(b"audio"));
    let native = SpeechCall {
        voice: named_voice("Kore"),
        ..base_call("vertex_ai/Gemini-2.5-preview-tts")
    };
    harness.run_ok(&native);
    assert!(harness.completions.calls().is_empty());
    let executed = harness.operations.take();
    assert_eq!(executed.len(), 1);
    assert_eq!(executed[0].kind, SpeechOperationKind::Dispatch);
    assert_eq!(
        executed[0].voice,
        Some(SpeechVoice::Name(String::from("Kore")))
    );
}
