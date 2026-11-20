use betterlite_config::sections::speech_call::Settings;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_openai::openai_base_config::Object;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;

use super::super::branches::take_azure_ad_token;
use super::super::call::SpeechCall;
use super::super::error::SpeechFailure;
use super::support::*;

fn keyed(key: Option<&str>) -> SpeechCall {
    SpeechCall {
        voice: named_voice("alloy"),
        api_key: key.map(SecretString::from),
        ..base_call("openai/tts-1")
    }
}

// the one request the OpenAI branch sends under this settings and environment
fn request(call: &SpeechCall, settings: Settings, variables: &[(&str, &str)]) -> OutboundRequest {
    let harness = Harness::new(
        settings,
        RecordingTransport::ok("audio/mpeg", &[1]),
        variables,
    );
    harness.run_ok(call);
    let mut sent = harness.transport.sent();
    assert_eq!(sent.len(), 1);
    sent.remove(0)
}

#[test]
fn openai_key_tiers_skip_empty_values() {
    let global = loaded_settings("litellm_settings: {api_key: sk-global}\n", &[]);
    assert_eq!(
        header(
            &request(&keyed(Some("sk-call")), global.clone(), &[]),
            "authorization"
        )
        .as_deref(),
        Some("Bearer sk-call")
    );
    assert_eq!(
        header(&request(&keyed(Some("")), global, &[]), "authorization").as_deref(),
        Some("Bearer sk-global")
    );
    let openai_key = loaded_settings("litellm_settings: {openai_key: sk-openai}\n", &[]);
    assert_eq!(
        header(&request(&keyed(Some("")), openai_key, &[]), "authorization").as_deref(),
        Some("Bearer sk-openai")
    );
    let variables = [("OPENAI_API_KEY", "sk-env")];
    assert_eq!(
        header(
            &request(
                &keyed(None),
                loaded_settings(SOURCE, &variables),
                &variables
            ),
            "authorization"
        )
        .as_deref(),
        Some("Bearer sk-env")
    );
}

#[test]
fn openai_reports_the_missing_key_as_an_sdk_error() {
    let harness = Harness::new(
        loaded_settings(SOURCE, &[]),
        RecordingTransport::ok("audio/mpeg", &[1]),
        &[],
    );
    let failure = harness.run_err(&keyed(None));
    assert_eq!(
        failure.message,
        "The api_key client option must be set either by passing api_key to the client or by setting the OPENAI_API_KEY environment variable"
    );
    assert_eq!(
        failure.failure,
        SpeechFailure::Class {
            class: "OpenAIError"
        }
    );
    assert!(harness.transport.sent().is_empty());
}

#[test]
fn openai_base_tiers_skip_empty_values() {
    let global = loaded_settings(
        "litellm_settings: {api_base: 'https://global.test/v1'}\n",
        &[],
    );
    assert_eq!(
        request(&keyed(Some("sk")), global.clone(), &[])
            .url
            .to_string(),
        "https://global.test/v1/audio/speech"
    );
    let call_base = SpeechCall {
        api_base: Some(String::from("https://call.test/v1")),
        ..keyed(Some("sk"))
    };
    assert_eq!(
        request(&call_base, global, &[]).url.to_string(),
        "https://call.test/v1/audio/speech"
    );
    let base_url = SpeechCall {
        base_url: Some(String::from("https://alias.test/v1")),
        ..keyed(Some("sk"))
    };
    assert_eq!(
        request(&base_url, loaded_settings(SOURCE, &[]), &[])
            .url
            .to_string(),
        "https://alias.test/v1/audio/speech"
    );
    let variables = [
        ("OPENAI_API_KEY", "sk-env"),
        ("OPENAI_BASE_URL", "https://base-url.test/v1"),
    ];
    assert_eq!(
        request(
            &keyed(None),
            loaded_settings(SOURCE, &variables),
            &variables
        )
        .url
        .to_string(),
        "https://base-url.test/v1/audio/speech"
    );
    let api_base = [
        ("OPENAI_API_KEY", "sk-env"),
        ("OPENAI_API_BASE", "https://api-base.test/v1"),
    ];
    assert_eq!(
        request(&keyed(None), loaded_settings(SOURCE, &api_base), &api_base)
            .url
            .to_string(),
        "https://api-base.test/v1/audio/speech"
    );
    let empty = [
        ("OPENAI_API_KEY", "sk-env"),
        ("OPENAI_BASE_URL", ""),
        ("OPENAI_API_BASE", ""),
    ];
    assert_eq!(
        request(&keyed(None), loaded_settings(SOURCE, &empty), &empty)
            .url
            .to_string(),
        "https://api.openai.com/v1/audio/speech"
    );
}

#[test]
fn openai_organization_and_project_are_not_forwarded() {
    let variables = [("OPENAI_API_KEY", "sk-env")];
    let harness = Harness::new(
        loaded_settings(SOURCE, &variables),
        RecordingTransport::ok("audio/mpeg", &[1]),
        &variables,
    );
    let native = SpeechCall {
        organization: Some(String::from("org-1")),
        project: Some(String::from("proj-1")),
        ..keyed(None)
    };
    harness.run_ok(&native);
    let sent = harness.transport.sent();
    assert_eq!(header(&sent[0], "openai-organization"), None);
    assert_eq!(header(&sent[0], "openai-project"), None);
}

#[test]
fn azure_credentials_fill_the_operation() {
    let variables = [("AZURE_AD_TOKEN", "env-token")];
    let harness = Harness::new(
        loaded_settings(SOURCE, &variables),
        RecordingTransport::ok("audio/mpeg", &[1]),
        &variables,
    );
    let native = SpeechCall {
        voice: named_voice("alloy"),
        api_base: Some(String::from("https://azure.test")),
        api_version: Some(String::from("2024-06-01")),
        api_key: Some(SecretString::from("sk-azure")),
        organization: Some(String::from("org-1")),
        kwargs: object(
            r#"{"callbacks": ["cb"], "extra_headers": {"x-header": "yes"},
                "azure_ad_token_provider": {"kind": "provider"}}"#,
        ),
        ..base_call("azure/my-deployment")
    };
    harness.run_ok(&native);
    assert!(harness.transport.sent().is_empty());
    let executed = harness.operations.take();
    assert_eq!(executed.len(), 1);
    let operation = &executed[0];
    assert_eq!(operation.api_base.as_deref(), Some("https://azure.test"));
    assert_eq!(operation.api_version.as_deref(), Some("2024-06-01"));
    assert_eq!(operation.api_key.as_deref(), Some("sk-azure"));
    assert_eq!(operation.organization.as_deref(), Some("org-1"));
    assert_eq!(operation.azure_ad_token.as_deref(), Some("env-token"));
    assert_eq!(
        operation.azure_ad_token_provider,
        Some(serde_json::json!({"kind": "provider"}))
    );
    assert_eq!(
        operation.optional_params.get("extra_headers"),
        Some(&serde_json::json!({"x-header": "yes"}))
    );
    assert_eq!(
        operation.kwargs.get("callbacks"),
        Some(&serde_json::json!(["cb"]))
    );
    assert_eq!(operation.timeout_seconds.to_bits(), 6000.0_f64.to_bits());
    assert_eq!(operation.max_retries, 2_u32);
}

#[test]
fn azure_empty_call_values_fall_through_to_the_settings() {
    let harness = Harness::new(
        loaded_settings(
            "litellm_settings: {api_base: 'https://global.test', api_version: '2024-10-01', api_key: sk-global}\n",
            &[],
        ),
        RecordingTransport::ok("audio/mpeg", &[1]),
        &[],
    );
    let native = SpeechCall {
        voice: named_voice("alloy"),
        api_base: Some(String::new()),
        api_version: Some(String::new()),
        api_key: Some(SecretString::from("")),
        ..base_call("azure/my-deployment")
    };
    harness.run_ok(&native);
    let executed = harness.operations.take();
    assert_eq!(executed[0].api_base.as_deref(), Some("https://global.test"));
    assert_eq!(executed[0].api_version.as_deref(), Some("2024-10-01"));
    assert_eq!(executed[0].api_key.as_deref(), Some("sk-global"));
}

#[test]
fn azure_environment_tiers_fill_the_operation() {
    let variables = [
        ("AZURE_API_BASE", "https://env-azure.test"),
        ("AZURE_API_VERSION", "2024-12-01"),
        ("AZURE_OPENAI_API_KEY", "sk-env-azure"),
    ];
    let harness = Harness::new(
        loaded_settings(SOURCE, &variables),
        RecordingTransport::ok("audio/mpeg", &[1]),
        &variables,
    );
    let native = SpeechCall {
        voice: named_voice("alloy"),
        ..base_call("azure/my-deployment")
    };
    harness.run_ok(&native);
    let executed = harness.operations.take();
    assert_eq!(
        executed[0].api_base.as_deref(),
        Some("https://env-azure.test")
    );
    assert_eq!(executed[0].api_version.as_deref(), Some("2024-12-01"));
    assert_eq!(executed[0].api_key.as_deref(), Some("sk-env-azure"));
}

#[test]
fn azure_ad_token_prefers_the_options_body() {
    let settings = loaded_settings(SOURCE, &[("AZURE_AD_TOKEN", "env-token")]);
    let mut params = object(r#"{"extra_body": {"azure_ad_token": "body-token", "other": 1}}"#);
    assert_eq!(
        take_azure_ad_token(&mut params, &settings)
            .map(|token| token.expose_secret().to_owned())
            .as_deref(),
        Some("body-token")
    );
    assert_eq!(
        Value::Object(params),
        serde_json::json!({"extra_body": {"other": 1}})
    );
    let mut empty = object(r#"{"extra_body": {"azure_ad_token": ""}}"#);
    assert_eq!(
        take_azure_ad_token(&mut empty, &settings)
            .map(|token| token.expose_secret().to_owned())
            .as_deref(),
        Some("env-token")
    );
    let mut without = Object::new();
    assert!(take_azure_ad_token(&mut without, &settings).is_some());
    assert!(take_azure_ad_token(&mut Object::new(), &loaded_settings(SOURCE, &[])).is_none());
}

#[test]
fn the_resolved_key_reaches_litellm_params() {
    let variables = [("MISTRAL_API_KEY", "sk-mistral")];
    let harness = Harness::new(
        loaded_settings(SOURCE, &variables),
        RecordingTransport::ok("audio/mpeg", &[1]),
        &variables,
    );
    let native = SpeechCall {
        voice: named_voice("alice"),
        ..base_call("mistral/mistral-small")
    };
    harness.run_ok(&native);
    let executed = harness.operations.take();
    assert_eq!(
        executed[0].litellm_params.get("api_key"),
        Some(&Value::String(String::from("sk-mistral")))
    );
    assert_eq!(executed[0].api_key, None);
    assert_eq!(
        executed[0].litellm_params.get("api_base"),
        Some(&Value::String(String::from("https://api.mistral.ai/v1")))
    );
}
