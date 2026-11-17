use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::enums::PiiAction;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::{LitellmParamsMode, Mode, ModeTarget};
use betterlite_config::sections::presidio_runtime::Settings as RuntimeSettings;
use tempfile::NamedTempFile;

use super::support::{GUARDRAIL_NAME, args, build, guard, guard_args, quiet};
use crate::guardrail_initializers::presidio::{PresidioArgs, PresidioRole};
use crate::presidio_actions::error::PresidioActionsError;
use crate::presidio_analysis::{DEFAULT_ANALYZE_TIMEOUT, DEFAULT_CHUNK_SIZE_BYTES};
use crate::presidio_runtime::PresidioDefaults;
use crate::presidio_runtime::error::PresidioRuntimeError;

fn failing(args: &PresidioArgs) -> PresidioRuntimeError {
    build(
        args,
        PresidioDefaults::default(),
        &RuntimeSettings::default(),
        false,
        Some(args.role),
    )
    .err()
    .expect("the constructor error")
}

fn temp_file(contents: &str) -> NamedTempFile {
    let file = NamedTempFile::new().expect("the temp file");
    std::fs::write(file.path(), contents).expect("the file contents");
    file
}

fn missing_path() -> String {
    format!(
        "{}/presidio-runtime-no-such-recognizer.json",
        env!("CARGO_MANIFEST_DIR")
    )
}

pub(super) fn defaults_are_applied() {
    let server = quiet();
    let guard = guard(&guard_args(&server));
    let settings = &guard.analyzer.settings;
    assert_eq!(settings.guardrail_name, GUARDRAIL_NAME);
    assert_eq!(settings.analyzer_api_base, format!("{}/", server.base));
    assert_eq!(settings.language, "en");
    assert!(!settings.output_parse_pii);
    assert!(!settings.apply_to_output);
    assert!(settings.score_thresholds.is_empty());
    assert!(settings.entities_deny_list.is_empty());
    assert!(settings.pii_entities.is_empty());
    assert!(settings.ad_hoc_recognizers.is_none());
    assert!(settings.mock_redacted_text.is_none());
    assert_eq!(settings.chunk_size_bytes, DEFAULT_CHUNK_SIZE_BYTES);
    assert_eq!(settings.timeout, DEFAULT_ANALYZE_TIMEOUT);
    assert_eq!(
        guard.anonymizer.settings.anonymizer_api_base,
        format!("{}/", server.base)
    );
    assert_eq!(guard.anonymizer.settings.timeout, DEFAULT_ANALYZE_TIMEOUT);
    assert!(guard.pii_entities_config.is_empty());
    assert!(!guard.global_output_parse_pii);
    assert_eq!(guard.callback_role, Some(PresidioRole::Scan));
}

pub(super) fn configured_values_are_applied() {
    let server = quiet();
    let mut args = guard_args(&server);
    args.output_parse_pii = Some(true);
    args.apply_to_output = true;
    args.presidio_language = Some("de".to_owned());
    args.presidio_score_thresholds = Some(BTreeMap::from([("PHONE_NUMBER".to_owned(), 0.5)]));
    args.presidio_entities_deny_list = Some(vec!["US_SSN".to_owned()]);
    args.pii_entities_config = Some(BTreeMap::from([("US_SSN".to_owned(), PiiAction::Block)]));
    args.analyze_chunk_size_bytes = Some(10);
    args.timeout = Some(12.5);
    args.mock_redacted_text = Some(BTreeMap::from([(
        "text".to_owned(),
        GuardrailValue::Text("redacted".to_owned()),
    )]));
    let settings = RuntimeSettings {
        output_parse_pii: true,
    };
    let guard = super::support::guard_with(&args, &settings);
    let analyzer = &guard.analyzer.settings;
    assert_eq!(analyzer.language, "de");
    assert!(analyzer.output_parse_pii);
    assert!(analyzer.apply_to_output);
    assert_eq!(
        analyzer.score_thresholds,
        BTreeMap::from([("PHONE_NUMBER".to_owned(), 0.5)])
    );
    assert_eq!(analyzer.entities_deny_list, vec!["US_SSN".to_owned()]);
    assert_eq!(analyzer.pii_entities, vec!["US_SSN".to_owned()]);
    assert_eq!(analyzer.chunk_size_bytes, 10_usize);
    assert_eq!(analyzer.timeout, Duration::from_millis(12_500));
    assert_eq!(
        analyzer
            .mock_redacted_text
            .as_ref()
            .map(|mock| mock.text.as_str()),
        Some("redacted")
    );
    assert!(guard.pii_entities_config.contains_key("US_SSN"));
    assert!(guard.global_output_parse_pii);
}

pub(super) fn hook_widens_for_a_roleless_output_callback() {
    let server = quiet();
    let mut args = guard_args(&server);
    args.output_parse_pii = Some(true);
    let guard = build(
        &args,
        PresidioDefaults::default(),
        &RuntimeSettings::default(),
        false,
        None,
    )
    .expect("the roleless guard");
    assert_eq!(
        guard.base.settings.event_hook,
        Some(LitellmParamsMode::Many(vec![
            "pre_call".to_owned(),
            "post_call".to_owned()
        ]))
    );
}

pub(super) fn hook_widens_for_a_roleless_output_mask_callback() {
    let server = quiet();
    let mut args = guard_args(&server);
    args.apply_to_output = true;
    let guard = build(
        &args,
        PresidioDefaults::default(),
        &RuntimeSettings::default(),
        false,
        None,
    )
    .expect("the roleless guard");
    assert_eq!(
        guard.base.settings.event_hook,
        Some(LitellmParamsMode::Many(vec![
            "pre_call".to_owned(),
            "post_call".to_owned()
        ]))
    );
}

pub(super) fn hook_widening_appends_to_a_list() {
    let server = quiet();
    let mut args = guard_args(&server);
    args.output_parse_pii = Some(true);
    args.event_hook = LitellmParamsMode::Many(vec!["pre_call".to_owned()]);
    let guard = build(
        &args,
        PresidioDefaults::default(),
        &RuntimeSettings::default(),
        false,
        None,
    )
    .expect("the roleless guard");
    assert_eq!(
        guard.base.settings.event_hook,
        Some(LitellmParamsMode::Many(vec![
            "pre_call".to_owned(),
            "post_call".to_owned()
        ]))
    );
}

pub(super) fn hook_widening_keeps_a_list_that_has_post_call() {
    let server = quiet();
    let mut args = guard_args(&server);
    args.output_parse_pii = Some(true);
    args.event_hook = LitellmParamsMode::Many(vec!["pre_call".to_owned(), "post_call".to_owned()]);
    let guard = build(
        &args,
        PresidioDefaults::default(),
        &RuntimeSettings::default(),
        false,
        None,
    )
    .expect("the roleless guard");
    assert_eq!(guard.base.settings.event_hook, Some(args.event_hook));
}

pub(super) fn hook_widening_skips_a_callback_with_a_role() {
    let server = quiet();
    let mut args = guard_args(&server);
    args.output_parse_pii = Some(true);
    let guard = guard(&args);
    assert_eq!(
        guard.base.settings.event_hook,
        Some(LitellmParamsMode::One("pre_call".to_owned()))
    );
}

pub(super) fn hook_widening_skips_a_roleless_callback_without_output() {
    let server = quiet();
    let args = guard_args(&server);
    let guard = build(
        &args,
        PresidioDefaults::default(),
        &RuntimeSettings::default(),
        false,
        None,
    )
    .expect("the roleless guard");
    assert_eq!(
        guard.base.settings.event_hook,
        Some(LitellmParamsMode::One("pre_call".to_owned()))
    );
}

pub(super) fn hook_widening_keeps_a_tagged_mode() {
    let server = quiet();
    let mut args = guard_args(&server);
    args.output_parse_pii = Some(true);
    args.event_hook = LitellmParamsMode::Tagged(Mode {
        tags: BTreeMap::from([("beta".to_owned(), ModeTarget::One("pre_call".to_owned()))]),
        default: None,
    });
    let guard = build(
        &args,
        PresidioDefaults::default(),
        &RuntimeSettings::default(),
        false,
        None,
    )
    .expect("the roleless guard");
    assert_eq!(guard.base.settings.event_hook, Some(args.event_hook));
}

pub(super) fn logging_only_sets_the_event_hook() {
    let server = quiet();
    let args = guard_args(&server);
    let guard = build(
        &args,
        PresidioDefaults::default(),
        &RuntimeSettings::default(),
        true,
        None,
    )
    .expect("the logging guard");
    assert_eq!(
        guard.base.settings.event_hook,
        Some(LitellmParamsMode::One("logging_only".to_owned()))
    );
}

pub(super) fn missing_recognizer_file_reports_the_path() {
    let mut args = args("", "");
    let path = missing_path();
    args.presidio_ad_hoc_recognizers = Some(path.clone());
    let error = failing(&args);
    assert_eq!(
        error.to_string(),
        format!("File not found. file_path={path}")
    );
    assert!(matches!(error, PresidioRuntimeError::FileNotFound { path: found } if found == path));
}

pub(super) fn invalid_recognizer_json_reports_the_decode_error() {
    let file = temp_file("{not json");
    let path = file.path().display().to_string();
    let mut args = args("", "");
    args.presidio_ad_hoc_recognizers = Some(path.clone());
    let error = failing(&args);
    let message = error.to_string();
    assert!(message.starts_with("Error decoding JSON file: "));
    assert!(message.ends_with(&format!(", file_path={path}")));
}

pub(super) fn unreadable_recognizer_file_reports_the_error() {
    let directory = tempfile::tempdir().expect("the temp directory");
    let path = directory.path().display().to_string();
    let mut args = args("", "");
    args.presidio_ad_hoc_recognizers = Some(path.clone());
    let error = failing(&args);
    let message = error.to_string();
    assert!(message.starts_with("An error occurred: "));
    assert!(message.ends_with(&format!(", file_path={path}")));
}

pub(super) fn recognizer_file_that_is_not_a_list_reports_the_decode_error() {
    let file = temp_file(r#"{"name": "a recognizer"}"#);
    let path = file.path().display().to_string();
    let mut args = args("", "");
    args.presidio_ad_hoc_recognizers = Some(path.clone());
    let error = failing(&args);
    assert!(matches!(error, PresidioRuntimeError::Decode { .. }));
    assert!(error.to_string().starts_with("Error decoding JSON file: "));
}

pub(super) fn recognizer_file_is_read_before_the_api_bases() {
    let mut args = args("", "");
    args.presidio_ad_hoc_recognizers = Some(missing_path());
    let error = failing(&args);
    assert!(matches!(error, PresidioRuntimeError::FileNotFound { .. }));
}

pub(super) fn missing_bases_report_the_analyzer_first() {
    let mut args = args("", "");
    args.presidio_analyzer_api_base = None;
    args.presidio_anonymizer_api_base = None;
    let error = failing(&args);
    assert!(matches!(
        error,
        PresidioRuntimeError::Actions(PresidioActionsError::MissingAnalyzerApiBase)
    ));
}

pub(super) fn a_missing_anonymizer_base_is_reported() {
    let server = quiet();
    let mut args = guard_args(&server);
    args.presidio_anonymizer_api_base = None;
    let error = failing(&args);
    assert!(matches!(
        error,
        PresidioRuntimeError::Actions(PresidioActionsError::MissingAnonymizerApiBase)
    ));
}

pub(super) fn defaults_supply_the_api_bases() {
    let server = quiet();
    let mut args = guard_args(&server);
    args.presidio_analyzer_api_base = None;
    args.presidio_anonymizer_api_base = None;
    let guard = build(
        &args,
        super::support::defaults(&server.base, &server.base),
        &RuntimeSettings::default(),
        false,
        Some(PresidioRole::Scan),
    )
    .expect("the guard");
    assert_eq!(
        guard.analyzer.settings.analyzer_api_base,
        format!("{}/", server.base)
    );
    assert_eq!(
        guard.anonymizer.settings.anonymizer_api_base,
        format!("{}/", server.base)
    );
}

pub(super) fn an_explicit_base_wins_over_the_default() {
    let server = quiet();
    let mut args = guard_args(&server);
    args.presidio_analyzer_api_base = Some("http://argument.test".to_owned());
    let guard = build(
        &args,
        super::support::defaults(&server.base, &server.base),
        &RuntimeSettings::default(),
        false,
        Some(PresidioRole::Scan),
    )
    .expect("the guard");
    assert_eq!(
        guard.analyzer.settings.analyzer_api_base,
        "http://argument.test/"
    );
    assert_eq!(
        guard.anonymizer.settings.anonymizer_api_base,
        format!("{}/", server.base)
    );
}

pub(super) fn hook_widening_keeps_a_post_call_hook() {
    let server = quiet();
    let mut args = guard_args(&server);
    args.output_parse_pii = Some(true);
    args.event_hook = LitellmParamsMode::One("post_call".to_owned());
    let guard = build(
        &args,
        PresidioDefaults::default(),
        &RuntimeSettings::default(),
        false,
        None,
    )
    .expect("the roleless guard");
    assert_eq!(
        guard.base.settings.event_hook,
        Some(LitellmParamsMode::One("post_call".to_owned()))
    );
}

pub(super) fn unrepresentable_timeout() {
    let server = quiet();
    let mut args = guard_args(&server);
    args.timeout = Some(-1.0);
    let error = failing(&args);
    assert!(matches!(
        error,
        PresidioRuntimeError::Timeout { seconds, .. } if seconds == -1.0
    ));
}
