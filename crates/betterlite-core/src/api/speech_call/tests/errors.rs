use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;

use super::super::call::SpeechCall;
use super::super::error::{SpeechCallError, SpeechFailure};
use super::support::*;

#[test]
fn openai_requires_a_string_voice() {
    let variables = [("OPENAI_API_KEY", "sk-env")];
    let harness = Harness::new(
        loaded_settings(SOURCE, &variables),
        RecordingTransport::ok("audio/mpeg", &[1]),
        &variables,
    );
    let missing = base_call("openai/tts-1");
    let missing_failure = harness.run_err(&missing);
    assert_eq!(
        missing_failure.message,
        "'voice' is required to be passed as a string for OpenAI TTS"
    );
    assert!(matches!(
        missing_failure.failure,
        SpeechFailure::Mapped(ErrorClass::BadRequest(_))
    ));
    assert!(harness.transport.sent().is_empty());
    let dict = SpeechCall {
        voice: Some(dict_voice(r#"{"voice": "alloy"}"#)),
        ..base_call("openai/tts-1")
    };
    let dict_failure = harness.run_err(&dict);
    assert_eq!(
        dict_failure.message,
        "'voice' is required to be passed as a string for OpenAI TTS"
    );
    let empty = SpeechCall {
        voice: Some(named_voice("")),
        ..base_call("openai/tts-1")
    };
    harness.run_ok(&empty);
    let sent = harness.transport.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent_json(&sent[0]).get("voice"),
        Some(&serde_json::json!(""))
    );
}

#[test]
fn azure_requires_a_string_voice_and_accepts_empty() {
    let harness = Harness::new(
        loaded_settings(SOURCE, &[]),
        RecordingTransport::ok("audio/mpeg", &[]),
        &[],
    );
    let missing = base_call("azure/my-deployment");
    let failure = harness.run_err(&missing);
    assert_eq!(
        failure.message,
        "'voice' is required to be passed as a string for Azure TTS"
    );
    assert!(matches!(
        failure.failure,
        SpeechFailure::Mapped(ErrorClass::BadRequest(_))
    ));
    assert!(harness.transport.sent().is_empty());
    // Azure OpenAI TTS reaches the provider through the operations seam, so the voice is what it receives
    let empty = SpeechCall {
        voice: Some(named_voice("")),
        ..base_call("azure/my-deployment")
    };
    harness.run_ok(&empty);
    let executed = harness.operations.take();
    assert_eq!(executed.len(), 1);
    assert_eq!(executed[0].voice, Some(named_voice("")));
}

#[test]
fn provider_status_errors_keep_the_mapped_fields() {
    let variables = [("OPENAI_API_KEY", "sk-env")];
    let body = br#"{"error":{"message":"slow down","type":"rate_limit_error","param":"model","code":"rate_limit"}}"#;
    let harness = Harness::new(
        loaded_settings(SOURCE, &variables),
        RecordingTransport::error(429, body),
        &variables,
    );
    let native = SpeechCall {
        voice: Some(named_voice("alloy")),
        ..base_call("openai/tts-1")
    };
    let failure = harness.run_err(&native);
    assert_eq!(
        failure.message,
        format!(
            "Error code: 429 - {}",
            serde_json::from_slice::<serde_json::Value>(body).expect("the error body")
        )
    );
    let SpeechFailure::Mapped(class) = &failure.failure else {
        panic!("a mapped provider status maps to a litellm class");
    };
    assert!(matches!(class, ErrorClass::RateLimit(_)));
    let metadata = class.metadata();
    assert_eq!(metadata.status_code, 429);
    assert_eq!(
        metadata.error_type,
        WireField::Text(String::from("throttling_error"))
    );
    assert_eq!(metadata.param, WireField::Text(String::from("model")));
    assert_eq!(metadata.openai_code.as_deref(), Some("429"));
}

#[test]
fn transport_failures_are_connection_errors() {
    let variables = [("OPENAI_API_KEY", "sk-env")];
    let harness = Harness::new(
        loaded_settings(SOURCE, &variables),
        BrokenTransport,
        &variables,
    );
    let native = SpeechCall {
        voice: Some(named_voice("alloy")),
        ..base_call("openai/tts-1")
    };
    let failure = harness.run_err(&native);
    assert_eq!(
        failure.failure,
        SpeechFailure::Class {
            class: "APIConnectionError"
        }
    );
    assert_eq!(failure.message, "the speech request could not be sent");
}

#[test]
fn resolution_failures_map_to_bad_request() {
    let harness = Harness::new(
        loaded_settings(SOURCE, &[]),
        RecordingTransport::ok("audio/mpeg", &[]),
        &[],
    );
    let native = SpeechCall {
        voice: Some(named_voice("alloy")),
        ..base_call("not-a-known-model")
    };
    let failure = harness.run_err(&native);
    let SpeechFailure::Mapped(class) = &failure.failure else {
        panic!("a resolution failure maps to a litellm class");
    };
    assert!(matches!(class, ErrorClass::BadRequest(_)));
    assert_eq!(class.metadata().status_code, 400);
}

#[test]
fn a_bridge_completion_failure_keeps_its_class() {
    let completions = ScriptedCompletions::failing(SpeechCallError::Provider {
        status: 500,
        message: String::from("boom"),
        error_type: Some(String::from("server_error")),
        param: None,
        code: None,
    });
    let harness = Harness::new(
        loaded_settings(SOURCE, &[]),
        RecordingTransport::ok("audio/wav", &[]),
        &[],
    )
    .completions(completions);
    let native = SpeechCall {
        voice: Some(named_voice("Kore")),
        ..base_call("gemini/gemini-2.5-flash-preview-tts")
    };
    let failure = harness.run_err(&native);
    assert_eq!(failure.message, "boom");
    let SpeechFailure::Mapped(class) = &failure.failure else {
        panic!("a provider failure maps to a litellm class");
    };
    assert!(matches!(class, ErrorClass::InternalServer(_)));
    assert_eq!(class.metadata().status_code, 500);
}

#[test]
fn internal_failures_are_plain_exceptions() {
    let completions = ScriptedCompletions::failing(SpeechCallError::Internal {
        message: String::from("bad transformation"),
    });
    let harness = Harness::new(
        loaded_settings(SOURCE, &[]),
        RecordingTransport::ok("audio/wav", &[]),
        &[],
    )
    .completions(completions);
    let native = SpeechCall {
        voice: Some(named_voice("Kore")),
        ..base_call("gemini/gemini-2.5-flash-preview-tts")
    };
    let failure = harness.run_err(&native);
    assert_eq!(failure.failure, SpeechFailure::Class { class: "Exception" });
    assert_eq!(failure.message, "bad transformation");
}

#[test]
fn unmapped_providers_raise_the_provider_list_exception() {
    let harness = Harness::new(
        loaded_settings(SOURCE, &[]),
        RecordingTransport::ok("audio/mpeg", &[]),
        &[],
    );
    let native = SpeechCall {
        voice: Some(named_voice("alice")),
        ..base_call("anthropic/claude-3")
    };
    let failure = harness.run_err(&native);
    assert_eq!(failure.failure, SpeechFailure::Class { class: "Exception" });
    assert!(
        failure
            .message
            .starts_with("Unable to map the custom llm provider=anthropic to a known provider=["),
        "{}",
        failure.message
    );
    assert!(harness.transport.sent().is_empty());
}

#[test]
fn anyscale_and_custom_providers_are_not_valid_llm_providers() {
    let harness = Harness::new(
        loaded_settings(SOURCE, &[]),
        RecordingTransport::ok("audio/mpeg", &[]),
        &[],
    );
    // LiteLLM knows the anyscale prefix; betterlite reaches Anyscale through its api_base
    let anyscale = SpeechCall {
        voice: Some(named_voice("alice")),
        api_base: Some(String::from("https://api.endpoints.anyscale.com/v1")),
        ..base_call("anyscale/mixtral-8x7b")
    };
    let anyscale_failure = harness.run_err(&anyscale);
    assert_eq!(
        anyscale_failure.failure,
        SpeechFailure::Class {
            class: "ValueError"
        }
    );
    assert_eq!(
        anyscale_failure.message,
        "'anyscale' is not a valid LlmProviders"
    );
    let custom = SpeechCall {
        voice: Some(named_voice("alice")),
        custom_llm_provider: Some(String::from("my-own")),
        ..base_call("model")
    };
    let custom_failure = harness.run_err(&custom);
    assert_eq!(
        custom_failure.message,
        "'my-own' is not a valid LlmProviders"
    );
    assert!(harness.transport.sent().is_empty());
}

#[test]
fn a_provider_never_reached_keeps_the_speech_errors() {
    let variables = [("OPENAI_API_KEY", "sk-env")];
    let harness = Harness::new(
        loaded_settings("litellm_settings: {api_base: 'not a url'}\n", &variables),
        RecordingTransport::ok("audio/mpeg", &[]),
        &variables,
    );
    let native = SpeechCall {
        voice: Some(named_voice("alloy")),
        ..base_call("openai/tts-1")
    };
    let failure = harness.run_err(&native);
    assert_eq!(
        failure.failure,
        SpeechFailure::Class {
            class: "OpenAIError"
        }
    );
    assert_eq!(failure.message, "the api_base is not a URL");
    assert!(harness.transport.sent().is_empty());
}
