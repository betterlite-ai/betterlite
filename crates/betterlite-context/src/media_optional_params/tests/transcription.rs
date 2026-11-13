#![cfg(test)]

use rstest::rstest;

use super::{FixtureMapper, params};
use crate::media_optional_params::error::MediaParamsError;
use crate::media_optional_params::transcription::{
    TranscriptionRequest, transcription_optional_params,
};
use crate::non_default_params::Params;
use crate::non_default_params::mapper::{DropPolicy, OptionalParamsMapper};

const SUPPORTED: FixtureMapper = FixtureMapper {
    supported: &["language", "temperature"],
    fail_status: None,
};

const FAILING: FixtureMapper = FixtureMapper {
    supported: &["language", "temperature"],
    fail_status: Some(422),
};

fn transcribe(
    provider: &str,
    passed: &str,
    drop_policy: DropPolicy,
    mapper: Option<&FixtureMapper>,
) -> Result<Params, MediaParamsError> {
    transcription_optional_params(&TranscriptionRequest {
        model: "m",
        custom_llm_provider: provider,
        params: &params(passed),
        drop_policy,
        mapper: mapper.map(|fixture| -> &dyn OptionalParamsMapper { fixture }),
    })
}

const OPENAI_EXPECTED: &str = r#"{"language": "de", "prompt": "p", "temperature": 0.2,
    "extra_body": {"prompt": "p", "temperature": 0.2, "top_k": 3}}"#;

#[rstest]
#[case::openai("openai", None)]
#[case::azure("azure", None)]
#[case::openai_ignores_a_mapper("openai", Some(&SUPPORTED))]
#[case::azure_ignores_a_mapper("azure", Some(&FAILING))]
fn media_optional_params_transcription_openai_uses_the_non_default_params(
    #[case] provider: &str,
    #[case] mapper: Option<&FixtureMapper>,
) {
    let passed = r#"{"language": "de", "prompt": "p", "temperature": 0.2, "response_format": null, "top_k": 3}"#;
    let result = transcribe(provider, passed, DropPolicy::default(), mapper).unwrap();
    assert_eq!(result, params(OPENAI_EXPECTED));
}

#[rstest]
#[case::zero_temperature(r#"{"temperature": 0}"#, r#"{"temperature": 0}"#)]
#[case::empty_language(r#"{"language": ""}"#, r#"{"language": ""}"#)]
#[case::null_language(r#"{"language": null}"#, "{}")]
#[case::granularities(
    r#"{"timestamp_granularities": ["word"]}"#,
    r#"{"timestamp_granularities": ["word"]}"#
)]
fn media_optional_params_transcription_set_values_are_non_default(
    #[case] passed: &str,
    #[case] expected: &str,
) {
    let mut without_extra = transcribe("openai", passed, DropPolicy::default(), None).unwrap();
    without_extra.remove("extra_body");
    assert_eq!(without_extra, params(expected));
}

#[test]
fn media_optional_params_transcription_mapper_receives_the_model_and_the_supported_params() {
    let passed = r#"{"language": "de", "temperature": 0.5}"#;
    let result = transcribe("other", passed, DropPolicy::default(), Some(&SUPPORTED)).unwrap();
    assert_eq!(
        result,
        params(
            r#"{"language": "de", "temperature": 0.5, "model": "m", "seen_drop": false, "seen_model": "m"}"#
        )
    );
}

#[test]
fn media_optional_params_transcription_compatible_provider_gets_extra_body() {
    let passed = r#"{"language": "de", "top_k": 3}"#;
    let result = transcribe("groq", passed, DropPolicy::default(), Some(&SUPPORTED)).unwrap();
    assert_eq!(
        result,
        params(
            r#"{"language": "de", "seen_drop": false, "seen_model": "m", "extra_body": {"top_k": 3}}"#
        )
    );
}

#[test]
fn media_optional_params_transcription_unsupported_param_is_rejected() {
    let passed = r#"{"language": "de", "prompt": "p"}"#;
    let error = transcribe("other", passed, DropPolicy::default(), Some(&SUPPORTED)).unwrap_err();
    assert!(matches!(
        &error,
        MediaParamsError::TranscriptionUnsupported { param, provider }
            if param == "prompt" && provider == "other"
    ));
    assert_eq!(error.status_code(), 400);
    assert_eq!(
        error.to_string(),
        "setting prompt is not supported by other"
    );
}

#[test]
fn media_optional_params_transcription_names_the_first_unsupported_param_in_key_order() {
    let passed = r#"{"response_format": "json", "prompt": "p"}"#;
    let error = transcribe("other", passed, DropPolicy::default(), Some(&SUPPORTED)).unwrap_err();
    assert!(matches!(
        &error,
        MediaParamsError::TranscriptionUnsupported { param, .. } if param == "prompt"
    ));
}

#[test]
fn media_optional_params_transcription_drop_params_false_still_rejects() {
    let policy = DropPolicy {
        drop_params: Some(false),
        litellm_drop_params: false,
    };
    let passed = r#"{"prompt": "p"}"#;
    let error = transcribe("other", passed, policy, Some(&SUPPORTED)).unwrap_err();
    assert!(matches!(
        error,
        MediaParamsError::TranscriptionUnsupported { .. }
    ));
}

#[rstest]
#[case::call_level_drop(Some(true), false, true)]
#[case::global_drop(None, true, false)]
#[case::global_drop_beats_a_call_level_false(Some(false), true, false)]
fn media_optional_params_transcription_drop_removes_unsupported_params(
    #[case] drop_params: Option<bool>,
    #[case] litellm_drop_params: bool,
    #[case] mapper_flag: bool,
) {
    let policy = DropPolicy {
        drop_params,
        litellm_drop_params,
    };
    let passed = r#"{"language": "de", "prompt": "p"}"#;
    let result = transcribe("other", passed, policy, Some(&SUPPORTED)).unwrap();
    // The passed prompt is still an unknown key, so it travels as a provider-specific param.
    assert_eq!(
        result,
        params(&format!(
            r#"{{"language": "de", "prompt": "p", "model": "m", "seen_drop": {mapper_flag}, "seen_model": "m"}}"#
        ))
    );
}

#[test]
fn media_optional_params_transcription_dropped_param_never_reaches_the_mapper() {
    let policy = DropPolicy {
        drop_params: Some(true),
        litellm_drop_params: false,
    };
    let mapper = FixtureMapper {
        supported: &["language"],
        fail_status: None,
    };
    let passed =
        r#"{"language": "de", "temperature": 0.1, "additional_drop_params": ["temperature"]}"#;
    let result = transcribe("other", passed, policy, Some(&mapper)).unwrap();
    assert_eq!(
        result,
        params(
            r#"{"language": "de", "model": "m", "seen_drop": true, "seen_model": "m", "additional_drop_params": ["temperature"]}"#
        )
    );
}

#[test]
fn media_optional_params_transcription_mapper_error_keeps_its_status() {
    let error = transcribe(
        "other",
        r#"{"language": "de"}"#,
        DropPolicy::default(),
        Some(&FAILING),
    )
    .unwrap_err();
    assert!(matches!(error, MediaParamsError::Mapper(_)));
    assert_eq!(error.status_code(), 422);
}

#[test]
fn media_optional_params_transcription_without_a_mapper_only_has_provider_params() {
    let passed = r#"{"language": "de", "temperature": 0.5, "top_k": 3}"#;
    let result = transcribe("other", passed, DropPolicy::default(), None).unwrap();
    assert_eq!(
        result,
        params(r#"{"temperature": 0.5, "top_k": 3, "model": "m"}"#)
    );
}

#[rstest]
#[case::list_of_names(
    r#"{"top_k": 3, "keep": 1, "additional_drop_params": ["top_k"]}"#,
    r#"{"keep": 1, "model": "m", "additional_drop_params": ["top_k"]}"#
)]
#[case::empty_list(
    r#"{"keep": 1, "additional_drop_params": []}"#,
    r#"{"keep": 1, "model": "m", "additional_drop_params": []}"#
)]
#[case::null(
    r#"{"keep": 1, "additional_drop_params": null}"#,
    r#"{"keep": 1, "model": "m"}"#
)]
#[case::missing(r#"{"keep": 1}"#, r#"{"keep": 1, "model": "m"}"#)]
fn media_optional_params_transcription_additional_drop_params_cases(
    #[case] passed: &str,
    #[case] expected: &str,
) {
    let result = transcribe("other", passed, DropPolicy::default(), None).unwrap();
    assert_eq!(result, params(expected));
}

#[rstest]
#[case::text(r#"{"additional_drop_params": "top_k"}"#)]
#[case::number(r#"{"additional_drop_params": 1}"#)]
#[case::list_with_a_number(r#"{"additional_drop_params": ["a", 1]}"#)]
#[case::list_with_a_null(r#"{"additional_drop_params": [null]}"#)]
#[case::object(r#"{"additional_drop_params": {"a": 1}}"#)]
fn media_optional_params_transcription_additional_drop_params_must_be_names(#[case] passed: &str) {
    let error = transcribe("other", passed, DropPolicy::default(), None).unwrap_err();
    assert!(matches!(
        error,
        MediaParamsError::WrongType("additional_drop_params", _)
    ));
    assert_eq!(error.status_code(), 400);
}
