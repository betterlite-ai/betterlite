#![cfg(test)]

use betterlite_config::sections::guardrail_params::base::UnreachableFallback;
use betterlite_config::sections::guardrail_params::mode::{Mode, ModeTarget};
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use rstest::rstest;

use super::{HeadroomEventHook, initialize_headroom};
use crate::third_party_init_a::error::ThirdPartyInitAError;
use crate::third_party_init_a::tests::{exposed, params, params_in_mode};

#[test]
fn headroom_refuses_a_missing_name() {
    let error = initialize_headroom(&params(""), None).unwrap_err();
    assert!(matches!(error, ThirdPartyInitAError::MissingHeadroomName));
    assert_eq!(
        error.to_string(),
        "Headroom guardrail requires a guardrail_name"
    );
}

#[test]
fn headroom_keeps_an_empty_name() {
    let built = initialize_headroom(&params(""), Some("")).unwrap();
    assert_eq!(built.name, "");
}

#[test]
fn headroom_takes_the_defaults_of_an_empty_config() {
    let built = initialize_headroom(&params(""), Some("headroom-guard")).unwrap();
    assert_eq!(built.name, "headroom-guard");
    assert_eq!(built.api_base, None);
    assert!(built.api_key.is_none());
    assert_eq!(built.model, None);
    assert_eq!(built.unreachable_fallback, UnreachableFallback::FailClosed);
    assert_eq!(built.timeout, None);
    assert!(built.ccr_retrieval);
    assert!(!built.default_on);
    assert_eq!(
        built.event_hook,
        HeadroomEventHook::One(GuardrailEventHooks::PreCall)
    );
}

#[test]
fn headroom_copies_the_configured_fields() {
    let built = initialize_headroom(
        &params(
            r#",
            "api_base": "https://headroom.test",
            "api_key": "headroom-key",
            "model": "gpt-test",
            "unreachable_fallback": "fail_open",
            "timeout": 11,
            "ccr_retrieval": false,
            "default_on": true
            "#,
        ),
        Some("headroom-guard"),
    )
    .unwrap();
    assert_eq!(built.api_base.as_deref(), Some("https://headroom.test"));
    assert_eq!(exposed(built.api_key.as_ref()), Some("headroom-key"));
    assert_eq!(built.model.as_deref(), Some("gpt-test"));
    assert_eq!(built.unreachable_fallback, UnreachableFallback::FailOpen);
    assert_eq!(built.timeout, Some(11.0_f64));
    assert!(!built.ccr_retrieval);
    assert!(built.default_on);
}

#[rstest]
#[case::absent("", true)]
#[case::null(r#", "ccr_retrieval": null"#, true)]
#[case::on(r#", "ccr_retrieval": true"#, true)]
#[case::off(r#", "ccr_retrieval": false"#, false)]
fn headroom_defaults_ccr_retrieval_to_on(#[case] extras: &str, #[case] expected: bool) {
    let built = initialize_headroom(&params(extras), Some("headroom-guard")).unwrap();
    assert_eq!(built.ccr_retrieval, expected);
}

#[test]
fn headroom_names_a_ccr_retrieval_of_a_wrong_type() {
    let error = initialize_headroom(
        &params(r#", "ccr_retrieval": "yes""#),
        Some("headroom-guard"),
    )
    .unwrap_err();
    assert!(matches!(
        &error,
        ThirdPartyInitAError::InvalidParam { name } if name == "ccr_retrieval"
    ));
}

#[test]
fn headroom_turns_a_single_mode_name_into_its_hook() {
    let built = initialize_headroom(&params_in_mode(r#""post_call""#, ""), Some("guard")).unwrap();
    assert_eq!(
        built.event_hook,
        HeadroomEventHook::One(GuardrailEventHooks::PostCall)
    );
}

#[test]
fn headroom_turns_a_list_of_mode_names_into_a_list_of_hooks() {
    let built = initialize_headroom(
        &params_in_mode(r#"["pre_call", "during_call", "post_call"]"#, ""),
        Some("guard"),
    )
    .unwrap();
    assert_eq!(
        built.event_hook,
        HeadroomEventHook::Many(vec![
            GuardrailEventHooks::PreCall,
            GuardrailEventHooks::DuringCall,
            GuardrailEventHooks::PostCall,
        ])
    );
}

#[test]
fn headroom_keeps_a_tagged_mode_as_it_is() {
    let built = initialize_headroom(
        &params_in_mode(
            r#"{"tags": {"finance": "pre_call", "legal": ["post_call"]}, "default": "during_call"}"#,
            "",
        ),
        Some("guard"),
    )
    .unwrap();
    let expected = Mode {
        tags: [
            ("finance".to_owned(), ModeTarget::One("pre_call".to_owned())),
            (
                "legal".to_owned(),
                ModeTarget::Many(vec!["post_call".to_owned()]),
            ),
        ]
        .into(),
        default: Some(ModeTarget::One("during_call".to_owned())),
    };
    assert_eq!(built.event_hook, HeadroomEventHook::Tagged(expected));
}

#[rstest]
#[case::single(r#""nonsense""#, "nonsense")]
#[case::in_a_list(r#"["pre_call", "nonsense"]"#, "nonsense")]
fn headroom_refuses_a_mode_name_that_is_not_an_event_hook(#[case] mode: &str, #[case] hook: &str) {
    let error = initialize_headroom(&params_in_mode(mode, ""), Some("guard")).unwrap_err();
    assert!(matches!(
        &error,
        ThirdPartyInitAError::InvalidEventHook { hook: named } if named == hook
    ));
    assert_eq!(
        error.to_string(),
        "nonsense is not a valid guardrail event hook"
    );
}

#[rstest]
#[case::pre_call("pre_call", GuardrailEventHooks::PreCall)]
#[case::post_call("post_call", GuardrailEventHooks::PostCall)]
#[case::during_call("during_call", GuardrailEventHooks::DuringCall)]
#[case::logging_only("logging_only", GuardrailEventHooks::LoggingOnly)]
#[case::pre_mcp_call("pre_mcp_call", GuardrailEventHooks::PreMcpCall)]
#[case::during_mcp_call("during_mcp_call", GuardrailEventHooks::DuringMcpCall)]
#[case::post_mcp_call("post_mcp_call", GuardrailEventHooks::PostMcpCall)]
#[case::realtime(
    "realtime_input_transcription",
    GuardrailEventHooks::RealtimeInputTranscription
)]
fn headroom_knows_every_event_hook_name(#[case] name: &str, #[case] expected: GuardrailEventHooks) {
    let built =
        initialize_headroom(&params_in_mode(&format!(r#""{name}""#), ""), Some("guard")).unwrap();
    assert_eq!(built.event_hook, HeadroomEventHook::One(expected));
}
