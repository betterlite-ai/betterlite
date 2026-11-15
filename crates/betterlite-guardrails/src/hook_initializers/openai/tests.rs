#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::decode::decode_litellm_params;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use secrecy::ExposeSecret as _;

use super::initialize_openai;

fn params(json: &str) -> LitellmParams {
    decode_litellm_params(json.as_bytes()).unwrap()
}

const REQUIRED: &str = r#""guardrail": "openai_moderation", "mode": "during_call""#;

#[test]
fn openai_requires_the_name() {
    let complete = params(r#"{"guardrail": "openai_moderation", "mode": "during_call"}"#);
    let unnamed = initialize_openai(&complete, None).unwrap_err();
    assert_eq!(
        unnamed.to_string(),
        "OpenAI Moderation: guardrail_name is required"
    );
    let empty = initialize_openai(&complete, Some("")).unwrap_err();
    assert_eq!(
        empty.to_string(),
        "OpenAI Moderation: guardrail_name is required"
    );
}

#[test]
fn openai_copies_the_listed_fields() {
    let args = initialize_openai(
        &params(&format!(
            r#"{{{REQUIRED}, "api_key": "openai-key", "api_base": "https://openai.example",
                "default_on": true, "model": "text-moderation-latest"}}"#
        )),
        Some("openai-check"),
    )
    .unwrap();
    assert_eq!(args.guardrail_name, "openai-check");
    assert_eq!(args.api_key.as_ref().unwrap().expose_secret(), "openai-key");
    assert_eq!(args.api_base.as_deref(), Some("https://openai.example"));
    assert!(args.default_on);
    assert_eq!(args.model.as_deref(), Some("text-moderation-latest"));
    assert_eq!(
        args.event_hook,
        LitellmParamsMode::One("during_call".to_owned())
    );
    let extras = &args.extras.0;
    assert_eq!(
        extras.get("api_key"),
        Some(&GuardrailValue::Text("openai-key".to_owned()))
    );
    assert_eq!(
        extras.get("event_hook"),
        Some(&GuardrailValue::Text("during_call".to_owned()))
    );
    assert_eq!(
        extras.get("default_on"),
        Some(&GuardrailValue::Boolean(true))
    );
    assert!(!extras.values().any(|value| *value == GuardrailValue::Null));
}

#[test]
fn openai_reads_each_streaming_field_from_optional_params_first() {
    let args = initialize_openai(
        &params(&format!(
            r#"{{{REQUIRED}, "streaming_end_of_stream_only": false,
                "streaming_sampling_rate": 3, "optional_params": {{
                    "streaming_end_of_stream_only": true,
                    "streaming_sampling_rate": 7}}}}"#
        )),
        Some("openai-check"),
    )
    .unwrap();
    assert_eq!(args.streaming_end_of_stream_only, Some(true));
    assert_eq!(args.streaming_sampling_rate, Some(7));
}

#[test]
fn openai_falls_back_to_the_params_when_optional_is_absent_or_null() {
    let from_params = initialize_openai(
        &params(&format!(
            r#"{{{REQUIRED}, "streaming_end_of_stream_only": false,
                "streaming_sampling_rate": 3}}"#
        )),
        Some("openai-check"),
    )
    .unwrap();
    assert_eq!(from_params.streaming_end_of_stream_only, Some(false));
    assert_eq!(from_params.streaming_sampling_rate, Some(3));
    let from_params_after_null = initialize_openai(
        &params(&format!(
            r#"{{{REQUIRED}, "streaming_end_of_stream_only": true,
                "streaming_sampling_rate": 9, "optional_params": {{
                    "streaming_end_of_stream_only": null,
                    "streaming_sampling_rate": null}}}}"#
        )),
        Some("openai-check"),
    )
    .unwrap();
    assert_eq!(
        from_params_after_null.streaming_end_of_stream_only,
        Some(true)
    );
    assert_eq!(from_params_after_null.streaming_sampling_rate, Some(9));
    let absent = initialize_openai(&params(&format!("{{{REQUIRED}}}")), Some("name")).unwrap();
    assert_eq!(absent.streaming_end_of_stream_only, None);
    assert_eq!(absent.streaming_sampling_rate, None);
}

#[test]
fn openai_refuses_a_wrongly_typed_streaming_field() {
    let error = initialize_openai(
        &params(&format!(
            r#"{{{REQUIRED}, "streaming_sampling_rate": "fast"}}"#
        )),
        Some("name"),
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "OpenAI Moderation: streaming_sampling_rate has an invalid value"
    );
}
