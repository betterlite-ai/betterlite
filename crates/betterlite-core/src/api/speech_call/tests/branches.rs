use betterlite_context::media_config_selection::text_to_speech::TextToSpeechConfigTag;
use betterlite_error::error::ErrorClass;
use betterlite_openai::api::speech_provider_contract::SpeechVoice;
use secrecy::SecretString;
use serde_json::Value;

use super::super::call::SpeechCall;
use super::super::error::SpeechFailure;
use super::super::operations::SpeechOperationKind;
use super::support::*;

// one provider branch over the scripted configs and operations, with no completion in reach
fn branch_harness(configs: Configs, operations: ScriptedOperations) -> Harness<RecordingTransport> {
    Harness::new(
        loaded_settings(SOURCE, &[]),
        RecordingTransport::ok("audio/wav", &[]),
        &[],
    )
    .configs(configs)
    .operations(operations)
}

#[test]
fn elevenlabs_maps_the_voice_and_keeps_the_query_params() {
    let harness = branch_harness(
        Configs::scripted(ScriptedConfig::keeping("{}")),
        ScriptedOperations::ok(b"audio"),
    );
    let native = SpeechCall {
        voice: named_voice("  id  "),
        api_base: Some(String::from("https://eleven.test")),
        api_key: Some(SecretString::from("sk-eleven")),
        kwargs: object(
            r#"{"tags": ["a"], "user": "u-1", "__elevenlabs_query_params__": {"enable_logging": false}}"#,
        ),
        ..base_call("elevenlabs/eleven_multilingual_v2")
    };
    let content = harness.run_ok(&native);
    assert_eq!(content.body, b"audio".to_vec());
    let executed = harness.operations.take();
    assert_eq!(executed.len(), 1);
    let operation = &executed[0];
    assert_eq!(operation.kind, SpeechOperationKind::Generic);
    assert_eq!(operation.provider, "elevenlabs");
    assert_eq!(
        operation.config_tag,
        Some(TextToSpeechConfigTag::ElevenLabs)
    );
    assert_eq!(operation.voice, Some(SpeechVoice::Name(String::from("id"))));
    assert_eq!(
        operation.litellm_params.get("__elevenlabs_voice_id__"),
        Some(&Value::String(String::from("id")))
    );
    assert_eq!(
        operation.litellm_params.get("__elevenlabs_query_params__"),
        Some(&serde_json::json!({"enable_logging": false}))
    );
    assert_eq!(operation.kwargs.get("__elevenlabs_query_params__"), None);
    assert_eq!(operation.kwargs.get("tags"), None);
    assert_eq!(
        operation.kwargs.get("user"),
        Some(&Value::String(String::from("u-1")))
    );
    assert_eq!(
        operation.litellm_params.get("api_base"),
        Some(&Value::String(String::from("https://eleven.test")))
    );
    assert_eq!(
        operation.litellm_params.get("api_key"),
        Some(&Value::String(String::from("sk-eleven")))
    );
    let mapping = harness.configs.mapping().expect("the scripted mapping");
    assert_eq!(
        mapping.calls()[0]
            .options
            .kwargs
            .get("__elevenlabs_query_params__"),
        Some(&serde_json::json!({"enable_logging": false}))
    );
}

#[test]
fn elevenlabs_drops_a_non_dict_query_and_rejects_blank_voices() {
    let harness = branch_harness(
        Configs::scripted(ScriptedConfig::keeping("{}")),
        ScriptedOperations::ok(b"audio"),
    );
    let native = SpeechCall {
        voice: named_voice("id"),
        kwargs: object(r#"{"__elevenlabs_query_params__": "junk"}"#),
        ..base_call("elevenlabs/eleven_multilingual_v2")
    };
    harness.run_ok(&native);
    let executed = harness.operations.take();
    assert_eq!(executed[0].kwargs.get("__elevenlabs_query_params__"), None);
    assert_eq!(
        executed[0]
            .litellm_params
            .get("__elevenlabs_query_params__"),
        None
    );
    let blank = SpeechCall {
        voice: named_voice("   "),
        ..base_call("elevenlabs/eleven_multilingual_v2")
    };
    let blank_failure = harness.run_err(&blank);
    assert_eq!(
        blank_failure.message,
        "'voice' must resolve to an ElevenLabs voice id for ElevenLabs TTS"
    );
    assert!(matches!(
        blank_failure.failure,
        SpeechFailure::Mapped(ErrorClass::BadRequest(_))
    ));
    let dict = SpeechCall {
        voice: dict_voice(r#"{"voice_id": "id"}"#),
        ..base_call("elevenlabs/eleven_multilingual_v2")
    };
    let dict_failure = harness.run_err(&dict);
    assert_eq!(
        dict_failure.message,
        "'voice' must resolve to an ElevenLabs voice id for ElevenLabs TTS"
    );
}

#[test]
fn minimax_selects_the_first_truthy_voice() {
    let harness = branch_harness(Configs::absent(), ScriptedOperations::ok(b"audio"));
    for (source, expected) in [
        (r#"{"voice_id": "v1", "id": "v2"}"#, Some("v1")),
        (r#"{"id": "v2", "name": "v3"}"#, Some("v2")),
        (r#"{"name": "v3"}"#, Some("v3")),
        (r#"{"voice_id": "", "id": "v2"}"#, Some("v2")),
        ("{}", None),
    ] {
        let native = SpeechCall {
            voice: dict_voice(source),
            ..base_call("minimax/speech-02-hd")
        };
        harness.run_ok(&native);
        let executed = harness.operations.take();
        assert_eq!(
            executed[0].voice,
            expected.map(|name| SpeechVoice::Name(name.to_owned())),
            "voice {source}"
        );
    }
    let native = SpeechCall {
        voice: named_voice("string-voice"),
        ..base_call("minimax/speech-02-hd")
    };
    harness.run_ok(&native);
    assert_eq!(
        harness.operations.take()[0].voice,
        named_voice("string-voice")
    );
}

#[test]
fn mistral_and_edenai_take_only_string_voices() {
    let harness = branch_harness(Configs::absent(), ScriptedOperations::ok(b"audio"));
    for model in ["mistral/mistral-small", "edenai/text-to-speech"] {
        let native = SpeechCall {
            voice: dict_voice(r#"{"voice": "alice"}"#),
            ..base_call(model)
        };
        harness.run_ok(&native);
        assert_eq!(
            harness.operations.take()[0].voice,
            None,
            "voice for {model}"
        );
    }
    let native = SpeechCall {
        voice: named_voice("alice"),
        ..base_call("mistral/mistral-small")
    };
    harness.run_ok(&native);
    assert_eq!(harness.operations.take()[0].voice, named_voice("alice"));
}

#[test]
fn edenai_records_the_resolved_api_base() {
    let harness = branch_harness(Configs::absent(), ScriptedOperations::ok(b"audio"));
    let native = SpeechCall {
        voice: named_voice("alice"),
        api_base: Some(String::from("https://eden.test/v1")),
        ..base_call("edenai/text-to-speech")
    };
    harness.run_ok(&native);
    assert_eq!(
        harness.operations.take()[0].litellm_params.get("api_base"),
        Some(&Value::String(String::from("https://eden.test/v1")))
    );
    let default_base = SpeechCall {
        voice: named_voice("alice"),
        ..base_call("edenai/text-to-speech")
    };
    harness.run_ok(&default_base);
    assert_eq!(
        harness.operations.take()[0].litellm_params.get("api_base"),
        Some(&Value::String(String::from("https://api.edenai.run/v3")))
    );
}

#[test]
fn polly_keeps_the_mapped_dict_voice() {
    let harness = branch_harness(
        Configs::scripted(ScriptedConfig::mapped(
            dict_voice(r#"{"voice": "Joanna"}"#),
            "{}",
        )),
        ScriptedOperations::ok(b"audio"),
    );
    let native = SpeechCall {
        voice: named_voice("alice"),
        ..base_call("aws_polly/generative")
    };
    harness.run_ok(&native);
    let executed = harness.operations.take();
    assert_eq!(executed[0].kind, SpeechOperationKind::Dispatch);
    assert_eq!(executed[0].voice, dict_voice(r#"{"voice": "Joanna"}"#));
}

#[test]
fn azure_speech_models_need_the_ava_config() {
    let native = SpeechCall {
        voice: dict_voice(r#"{"name": "en-US-JennyNeural"}"#),
        ..base_call("azure/speech/azure-text-to-speech")
    };
    let missing = branch_harness(Configs::absent(), ScriptedOperations::ok(b"audio"));
    let failure = missing.run_err(&native);
    assert_eq!(
        failure.message,
        "Azure Speech Service configuration not found"
    );
    assert!(matches!(
        failure.failure,
        SpeechFailure::Mapped(ErrorClass::BadRequest(_))
    ));
    assert!(missing.operations.take().is_empty());
    let harness = branch_harness(
        Configs::scripted(ScriptedConfig::keeping("{}")),
        ScriptedOperations::ok(b"audio"),
    );
    harness.run_ok(&native);
    let executed = harness.operations.take();
    assert_eq!(executed[0].kind, SpeechOperationKind::Dispatch);
    assert_eq!(
        executed[0].config_tag,
        Some(TextToSpeechConfigTag::AzureAVA)
    );
    assert_eq!(
        executed[0].voice,
        dict_voice(r#"{"name": "en-US-JennyNeural"}"#)
    );
}

#[test]
fn azure_ai_speech_models_never_reach_a_config() {
    let harness = branch_harness(
        Configs::scripted(ScriptedConfig::keeping("{}")),
        ScriptedOperations::ok(b"audio"),
    );
    let native = SpeechCall {
        voice: named_voice("alloy"),
        ..base_call("azure_ai/speech/azure-text-to-speech")
    };
    let failure = harness.run_err(&native);
    assert_eq!(
        failure.message,
        "Azure Speech Service configuration not found"
    );
    assert!(harness.operations.take().is_empty());
    assert!(
        harness
            .configs
            .mapping()
            .expect("the scripted mapping")
            .calls()
            .is_empty()
    );
}

#[test]
fn runwayml_needs_its_config_and_dispatches_with_it() {
    let native = SpeechCall {
        voice: named_voice("Maya"),
        ..base_call("runwayml/text-to-speech")
    };
    let missing = branch_harness(Configs::absent(), ScriptedOperations::ok(b"audio"));
    let failure = missing.run_err(&native);
    assert_eq!(
        failure.message,
        "RunwayML Text-to-Speech configuration not found"
    );
    let harness = branch_harness(
        Configs::scripted(ScriptedConfig::keeping(r#"{"voice": "Maya"}"#)),
        ScriptedOperations::ok(b"audio"),
    );
    harness.run_ok(&native);
    let executed = harness.operations.take();
    assert_eq!(executed[0].kind, SpeechOperationKind::Dispatch);
    assert_eq!(executed[0].provider, "runwayml");
    assert_eq!(
        executed[0].optional_params.get("voice"),
        Some(&Value::String(String::from("Maya")))
    );
}

#[test]
fn the_mapping_receives_drop_params_false_and_the_options() {
    let harness = branch_harness(
        Configs::scripted(ScriptedConfig::keeping(r#"{"mapped": true}"#)),
        ScriptedOperations::ok(b"audio"),
    );
    let native = SpeechCall {
        voice: named_voice("alloy"),
        response_format: Some(String::from("wav")),
        kwargs: object(r#"{"tags": ["a"], "user": "u-1"}"#),
        ..base_call("elevenlabs/eleven_multilingual_v2")
    };
    harness.run_ok(&native);
    let calls = harness
        .configs
        .mapping()
        .expect("the scripted mapping")
        .calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, "eleven_multilingual_v2");
    assert!(!calls[0].options.drop_params);
    assert_eq!(calls[0].options.voice, named_voice("alloy"));
    assert_eq!(calls[0].options.kwargs.get("tags"), None);
    assert_eq!(
        calls[0].options.kwargs.get("user"),
        Some(&Value::String(String::from("u-1")))
    );
    let executed = harness.operations.take();
    assert_eq!(
        executed[0].optional_params.get("response_format"),
        Some(&Value::String(String::from("wav")))
    );
    assert_eq!(
        executed[0].optional_params.get("mapped"),
        Some(&Value::Bool(true))
    );
}
