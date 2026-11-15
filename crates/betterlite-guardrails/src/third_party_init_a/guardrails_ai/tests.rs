#![cfg(test)]

use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use rstest::rstest;

use super::{GuardrailsAiInputFormat, initialize_guardrails_ai};
use crate::third_party_init_a::error::ThirdPartyInitAError;
use crate::third_party_init_a::tests::{exposed, params};

#[test]
fn guardrails_ai_refuses_a_missing_guard_name() {
    let error = initialize_guardrails_ai(&params(""), Some("ai-guard")).unwrap_err();
    assert!(matches!(error, ThirdPartyInitAError::MissingGuardName));
    assert_eq!(
        error.to_string(),
        "GuardrailsAIException - Please pass the Guardrails AI guard name via 'litellm_params::guard_name'"
    );
}

#[test]
fn guardrails_ai_refuses_a_null_guard_name() {
    let error = initialize_guardrails_ai(&params(r#", "guard_name": null"#), None).unwrap_err();
    assert!(matches!(error, ThirdPartyInitAError::MissingGuardName));
}

#[test]
fn guardrails_ai_keeps_an_empty_guard_name() {
    let built = initialize_guardrails_ai(&params(r#", "guard_name": """#), None).unwrap();
    assert_eq!(built.guard_name, "");
}

#[test]
fn guardrails_ai_takes_the_defaults_of_a_guard_name_alone() {
    let built = initialize_guardrails_ai(&params(r#", "guard_name": "pii""#), None).unwrap();
    assert_eq!(built.name, "");
    assert_eq!(built.guard_name, "pii");
    assert_eq!(built.api_base, None);
    assert!(built.api_key.is_none());
    assert_eq!(
        built.guardrails_ai_api_input_format,
        GuardrailsAiInputFormat::LlmOutput
    );
    assert_eq!(built.timeout, None);
    assert!(!built.default_on);
    assert_eq!(
        built.event_hook,
        LitellmParamsMode::One("pre_call".to_owned())
    );
}

#[test]
fn guardrails_ai_copies_the_configured_fields() {
    let built = initialize_guardrails_ai(
        &params(
            r#",
            "guard_name": "pii",
            "api_base": "https://guardrails.test",
            "api_key": "guardrails-key",
            "guardrails_ai_api_input_format": "inputs",
            "timeout": 7.5,
            "default_on": true
            "#,
        ),
        Some("ai-guard"),
    )
    .unwrap();
    assert_eq!(built.name, "ai-guard");
    assert_eq!(built.guard_name, "pii");
    assert_eq!(built.api_base.as_deref(), Some("https://guardrails.test"));
    assert_eq!(exposed(built.api_key.as_ref()), Some("guardrails-key"));
    assert_eq!(
        built.guardrails_ai_api_input_format,
        GuardrailsAiInputFormat::Inputs
    );
    assert_eq!(built.timeout, Some(7.5_f64));
    assert!(built.default_on);
}

#[rstest]
#[case::absent("", GuardrailsAiInputFormat::LlmOutput)]
#[case::null(
    r#", "guardrails_ai_api_input_format": null"#,
    GuardrailsAiInputFormat::LlmOutput
)]
#[case::llm_output(
    r#", "guardrails_ai_api_input_format": "llmOutput""#,
    GuardrailsAiInputFormat::LlmOutput
)]
#[case::inputs(
    r#", "guardrails_ai_api_input_format": "inputs""#,
    GuardrailsAiInputFormat::Inputs
)]
fn guardrails_ai_reads_the_input_format(
    #[case] extras: &str,
    #[case] expected: GuardrailsAiInputFormat,
) {
    let built =
        initialize_guardrails_ai(&params(&format!(r#", "guard_name": "pii"{extras}"#)), None)
            .unwrap();
    assert_eq!(built.guardrails_ai_api_input_format, expected);
}

#[rstest]
#[case::unknown_text(r#""other""#)]
#[case::number("3")]
fn guardrails_ai_names_an_input_format_of_the_wrong_kind(#[case] value: &str) {
    let error = initialize_guardrails_ai(
        &params(&format!(
            r#", "guard_name": "pii", "guardrails_ai_api_input_format": {value}"#
        )),
        None,
    )
    .unwrap_err();
    assert!(matches!(
        &error,
        ThirdPartyInitAError::InvalidParam { name } if name == "guardrails_ai_api_input_format"
    ));
}

#[test]
fn guardrails_ai_checks_the_guard_name_before_the_input_format() {
    let error = initialize_guardrails_ai(
        &params(r#", "guardrails_ai_api_input_format": "other""#),
        None,
    )
    .unwrap_err();
    assert!(matches!(error, ThirdPartyInitAError::MissingGuardName));
}
