use betterlite_openai::api::speech_provider_contract::SpeechVoice;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::openai_base_config::Object;
use rstest::rstest;
use serde_json::json;

use super::super::dispatch::HandlerFailure;
use super::super::error::RunwaySpeechError;
use super::support::{Run, dispatch_run, module_globals, settings, text_params, voice_params};

#[tokio::test]
async fn dispatch_prefers_the_explicit_pair() {
    let loaded = settings(&[]);
    let globals = module_globals(Some("https://global.test"), Some("sk-global"));
    let run = Run::default()
        .with_voice(SpeechVoice::Name(String::from("alloy")))
        .with_optional(voice_params("Maya"))
        .with_params(text_params(&[
            ("api_base", "https://params.test"),
            ("api_key", "sk-params"),
            ("custom", "kept"),
        ]))
        .with_explicit(Some("https://explicit.test"), Some("sk-explicit"))
        .with_headers(text_params(&[("X-Custom", "yes")]))
        .with_timeout(12.5)
        .speaking();
    let out = dispatch_run(&loaded, &globals, &run).await;
    assert_eq!(out.seen.len(), 1);
    let answer = out.answer.unwrap();
    assert_eq!(answer.get("voice"), Some(&json!("alloy")));
    assert_eq!(
        answer.get("optional_params"),
        Some(&json!({
            "runwayml_voice": {"type": "runway-preset", "presetId": "Maya"},
        }))
    );
    assert_eq!(
        answer.get("litellm_params"),
        Some(&json!({
            "api_base": "https://explicit.test",
            "api_key": "sk-explicit",
            "custom": "kept",
        }))
    );
    assert_eq!(
        answer.get("api_base"),
        Some(&json!("https://explicit.test"))
    );
    assert_eq!(answer.get("api_key"), Some(&json!("sk-explicit")));
    assert_eq!(answer.get("timeout_seconds"), Some(&json!(12.5)));
    assert_eq!(
        answer.get("extra_headers"),
        Some(&json!({"X-Custom": "yes"}))
    );
    assert_eq!(answer.get("aspeech"), Some(&json!(true)));
}

#[tokio::test]
async fn dispatch_falls_through_an_empty_explicit_pair() {
    let run = Run::default()
        .with_params(text_params(&[
            ("api_base", "https://params.test"),
            ("api_key", "sk-params"),
        ]))
        .with_explicit(Some(""), Some(""));
    let out = dispatch_run(&settings(&[]), &LiteLlmGlobals::default(), &run).await;
    let answer = out.answer.unwrap();
    assert_eq!(answer.get("api_base"), Some(&json!("https://params.test")));
    assert_eq!(answer.get("api_key"), Some(&json!("sk-params")));
}

#[tokio::test]
async fn dispatch_skips_empty_records_and_globals() {
    let loaded = settings(&[
        ("RUNWAYML_API_BASE", "https://env.test"),
        ("RUNWAYML_API_SECRET", "sk-secret"),
    ]);
    let globals = module_globals(Some(""), Some(""));
    let run = Run::default().with_params(text_params(&[("api_base", ""), ("api_key", "")]));
    let answer = dispatch_run(&loaded, &globals, &run).await.answer.unwrap();
    assert_eq!(answer.get("api_base"), Some(&json!("https://env.test")));
    assert_eq!(answer.get("api_key"), Some(&json!("sk-secret")));
}

#[tokio::test]
async fn dispatch_reads_the_module_globals_before_the_environment() {
    let loaded = settings(&[
        ("RUNWAYML_API_BASE", "https://env.test"),
        ("RUNWAYML_API_SECRET", "sk-secret"),
    ]);
    let globals = module_globals(Some("https://global.test"), Some("sk-global"));
    let answer = dispatch_run(&loaded, &globals, &Run::default())
        .await
        .answer
        .unwrap();
    assert_eq!(answer.get("api_base"), Some(&json!("https://global.test")));
    assert_eq!(answer.get("api_key"), Some(&json!("sk-global")));
}

#[tokio::test]
async fn dispatch_reads_the_environment_secret_before_the_key() {
    let loaded = settings(&[
        ("RUNWAYML_API_BASE", "https://env.test"),
        ("RUNWAYML_API_SECRET", "sk-secret"),
        ("RUNWAYML_API_KEY", "sk-key"),
    ]);
    let answer = dispatch_run(&loaded, &LiteLlmGlobals::default(), &Run::default())
        .await
        .answer
        .unwrap();
    assert_eq!(answer.get("api_base"), Some(&json!("https://env.test")));
    assert_eq!(answer.get("api_key"), Some(&json!("sk-secret")));
}

#[tokio::test]
async fn dispatch_uses_the_environment_key_when_no_secret_is_set() {
    let loaded = settings(&[("RUNWAYML_API_KEY", "sk-key")]);
    let answer = dispatch_run(&loaded, &LiteLlmGlobals::default(), &Run::default())
        .await
        .answer
        .unwrap();
    assert_eq!(
        answer.get("api_base"),
        Some(&json!("https://api.dev.runwayml.com"))
    );
    assert_eq!(answer.get("api_key"), Some(&json!("sk-key")));
}

#[tokio::test]
async fn dispatch_writes_a_null_key_record() {
    let run = Run::default().with_params(text_params(&[("custom", "kept")]));
    let answer = dispatch_run(&settings(&[]), &LiteLlmGlobals::default(), &run)
        .await
        .answer
        .unwrap();
    assert_eq!(
        answer.get("litellm_params"),
        Some(&json!({
            "custom": "kept",
            "api_base": "https://api.dev.runwayml.com",
            "api_key": null,
        }))
    );
    assert_eq!(answer.get("api_key"), Some(&json!(null)));
}

#[tokio::test]
async fn dispatch_treats_a_null_record_value_as_absent() {
    let loaded = settings(&[
        ("RUNWAYML_API_BASE", "https://env.test"),
        ("RUNWAYML_API_SECRET", "sk-secret"),
    ]);
    let params = json!({"api_base": null, "api_key": null})
        .as_object()
        .cloned()
        .unwrap();
    let run = Run::default().with_params(params);
    let answer = dispatch_run(&loaded, &LiteLlmGlobals::default(), &run)
        .await
        .answer
        .unwrap();
    assert_eq!(answer.get("api_base"), Some(&json!("https://env.test")));
    assert_eq!(answer.get("api_key"), Some(&json!("sk-secret")));
}

#[rstest]
#[case::base("api_base")]
#[case::key("api_key")]
#[tokio::test]
async fn dispatch_rejects_a_non_string_record_value(#[case] name: &'static str) {
    let mut record = Object::new();
    record.insert(name.to_owned(), json!(5));
    let run = Run::default().with_params(record);
    let out = dispatch_run(&settings(&[]), &LiteLlmGlobals::default(), &run).await;
    assert!(matches!(
        out.answer,
        Err(RunwaySpeechError::InvalidOption { option }) if option == name
    ));
    assert!(out.seen.is_empty());
}

#[tokio::test]
async fn dispatch_reports_a_handler_failure() {
    let run = Run::default().failing(HandlerFailure {
        class: String::from("RateLimitError"),
        status: Some(429),
        message: String::from("slow down"),
    });
    let out = dispatch_run(&settings(&[]), &LiteLlmGlobals::default(), &run).await;
    assert_eq!(out.seen.len(), 1);
    let error = out.answer.unwrap_err();
    assert_eq!(
        error.to_string(),
        String::from("slow down"),
        "the handler's own message is what the caller reads"
    );
    assert!(matches!(
        error,
        RunwaySpeechError::Handler {
            class,
            status: Some(429),
            message,
        } if class == "RateLimitError" && message == "slow down"
    ));
}
