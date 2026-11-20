#![cfg(test)]

use std::collections::BTreeMap;
use std::io;
use std::io::Write;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt;

use betterlite_config::sections::content_filter_records::{
    BlockedWord, ContentFilterAction, ContentFilterCategoryConfig, ContentFilterPattern,
    ContentFilterSeverity,
};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::base::OnViolation;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;

use super::error::ContentFilterBuildError;
use super::{
    CompetitorIntentChecker, ContentFilterGuardrail, GUARDRAIL_PROVIDER, KEYWORD_REDACTION_TAG,
    PATTERN_REDACTION_FORMAT, SUPPORTED_EVENT_HOOKS,
};
use crate::content_filter_init::args::ContentFilterArgs;
use crate::content_filter_inputs::images::{
    ImageDescriptionCompletion, ImageDescriptionReply, ImageDescriptionRequest,
    ImageDescriptionResponse,
};
use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::contract::GuardrailContract;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::GuardrailApiInputs;
use crate::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};
use crate::pattern_matching::PatternError;
use crate::pattern_matching::blocked_words::WordsError;

struct TestRouter {
    description: Option<String>,
}

#[derive(Debug, thiserror::Error)]
#[error("the router is unavailable")]
struct RouterFailure;

#[async_trait]
impl ImageDescriptionCompletion for TestRouter {
    type Error = RouterFailure;

    async fn describe(
        &self,
        _request: &ImageDescriptionRequest,
    ) -> Result<ImageDescriptionResponse, Self::Error> {
        let Some(description) = &self.description else {
            return Err(RouterFailure);
        };
        let reply = ImageDescriptionReply {
            content: Some(description.clone()),
        };
        Ok(ImageDescriptionResponse {
            choices: vec![reply],
        })
    }
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|poisoned| io::Error::other(poisoned.to_string()))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Captured {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

fn capture() -> (DefaultGuard, Captured) {
    let captured = Captured::default();
    let sink = captured.clone();
    let subscriber = fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .with_writer(move || sink.clone())
        .finish();
    (set_default(subscriber), captured)
}

fn args(name: &str) -> ContentFilterArgs {
    ContentFilterArgs {
        name: name.to_owned(),
        guardrail_id: None,
        policy_template: None,
        patterns: None,
        blocked_words: None,
        blocked_words_file: None,
        event_hook: LitellmParamsMode::One("pre_call".to_owned()),
        default_on: false,
        categories: None,
        severity_threshold: "medium".to_owned(),
        router_present: false,
        image_model: None,
        competitor_intent_config: None,
        end_session_after_n_fails: None,
        on_violation: None,
        realtime_violation_message: None,
    }
}

fn built(arguments: ContentFilterArgs) -> ContentFilterGuardrail<TestRouter> {
    ContentFilterGuardrail::new(arguments, None).unwrap()
}

fn built_with_router(
    arguments: ContentFilterArgs,
    router: TestRouter,
) -> ContentFilterGuardrail<TestRouter> {
    ContentFilterGuardrail::new(arguments, Some(router)).unwrap()
}

fn built_result(
    arguments: ContentFilterArgs,
) -> Result<ContentFilterGuardrail<TestRouter>, ContentFilterBuildError> {
    ContentFilterGuardrail::new(arguments, None)
}

fn expected_settings() -> GuardrailSettings {
    GuardrailSettings {
        guardrail_name: Some("content-filter".to_owned()),
        supported_event_hooks: Some(SUPPORTED_EVENT_HOOKS.to_vec()),
        event_hook: Some(LitellmParamsMode::One("pre_call".to_owned())),
        default_on: false,
        mask_request_content: false,
        mask_response_content: false,
        violation_message_template: None,
        end_session_after_n_fails: None,
        on_violation: None,
        realtime_violation_message: None,
        on_sensitive_data: None,
        sensitive_data_route_to_model: None,
        sticky_session_routing: true,
        run_in_parallel: false,
        scan_raw_request: false,
        only_scan_new_messages: false,
        timeout: None,
        switches: GuardrailSwitches {
            use_native_during_call_hook: false,
            use_native_lifecycle_hooks: false,
            records_own_information: true,
            uses_apply_guardrail_interface: true,
            deny_text_on_error: false,
        },
    }
}

fn competitor(json: &str) -> OpaqueParams {
    serde_json::from_str(json).unwrap()
}

fn regex_pattern(name: &str, source: &str, action: ContentFilterAction) -> ContentFilterPattern {
    ContentFilterPattern {
        pattern_type: "regex".to_owned(),
        pattern_name: None,
        pattern: Some(source.to_owned()),
        name: Some(name.to_owned()),
        action,
    }
}

fn blocked_word(keyword: &str, action: ContentFilterAction) -> BlockedWord {
    BlockedWord {
        keyword: keyword.to_owned(),
        action,
        description: None,
    }
}

fn described_word(keyword: &str, action: ContentFilterAction, description: &str) -> BlockedWord {
    BlockedWord {
        keyword: keyword.to_owned(),
        action,
        description: Some(description.to_owned()),
    }
}

fn category(name: &str) -> ContentFilterCategoryConfig {
    ContentFilterCategoryConfig {
        category: name.to_owned(),
        enabled: true,
        action: ContentFilterAction::Block,
        severity_threshold: ContentFilterSeverity::Medium,
        category_file: None,
    }
}

fn texts_inputs(texts: &[&str]) -> GuardrailApiInputs {
    GuardrailApiInputs {
        texts: Some(texts.iter().map(|text| (*text).to_owned()).collect()),
        ..GuardrailApiInputs::default()
    }
}

fn tool_call(arguments: &str) -> GuardrailValue {
    let function = BTreeMap::from([
        ("name".to_owned(), GuardrailValue::Text("lookup".to_owned())),
        (
            "arguments".to_owned(),
            GuardrailValue::Text(arguments.to_owned()),
        ),
    ]);
    GuardrailValue::Object(BTreeMap::from([
        (
            "type".to_owned(),
            GuardrailValue::Text("function".to_owned()),
        ),
        ("function".to_owned(), GuardrailValue::Object(function)),
    ]))
}

fn mcp_request_data() -> BTreeMap<String, GuardrailValue> {
    let arguments = BTreeMap::from([(
        "query".to_owned(),
        GuardrailValue::Text("secret stuff".to_owned()),
    )]);
    BTreeMap::from([
        (
            "mcp_tool_name".to_owned(),
            GuardrailValue::Text("search".to_owned()),
        ),
        (
            "mcp_arguments".to_owned(),
            GuardrailValue::Object(arguments),
        ),
    ])
}

fn intent(guardrail: &ContentFilterGuardrail<TestRouter>, text: &str) -> Option<String> {
    let checker = guardrail.competitor_intent_checker.as_ref()?;
    checker.run(text).intent
}

struct Applied {
    result: Result<GuardrailApiInputs, GuardrailFailure>,
    request_data: BTreeMap<String, GuardrailValue>,
}

async fn applied(
    guardrail: &ContentFilterGuardrail<TestRouter>,
    inputs: GuardrailApiInputs,
    input_type: GuardrailInputType,
) -> Applied {
    applied_with(guardrail, inputs, BTreeMap::new(), input_type).await
}

async fn applied_with(
    guardrail: &ContentFilterGuardrail<TestRouter>,
    inputs: GuardrailApiInputs,
    mut request_data: BTreeMap<String, GuardrailValue>,
    input_type: GuardrailInputType,
) -> Applied {
    let result = guardrail
        .apply_guardrail(inputs, &mut request_data, input_type, None)
        .await;
    Applied {
        result,
        request_data,
    }
}

fn field<'value>(value: &'value GuardrailValue, path: &[&str]) -> Option<&'value GuardrailValue> {
    let mut current = value;
    for key in path {
        let GuardrailValue::Object(fields) = current else {
            return None;
        };
        current = fields.get(*key)?;
    }
    Some(current)
}

fn text_field<'value>(value: &'value GuardrailValue, path: &[&str]) -> Option<&'value str> {
    match field(value, path) {
        Some(GuardrailValue::Text(text)) => Some(text),
        _ => None,
    }
}

fn int_field(value: &GuardrailValue, path: &[&str]) -> Option<i64> {
    match field(value, path) {
        Some(GuardrailValue::SignedInteger(number)) => Some(*number),
        _ => None,
    }
}

fn decimal_field(value: &GuardrailValue, path: &[&str]) -> Option<f64> {
    match field(value, path) {
        Some(GuardrailValue::Decimal(number)) => Some(*number),
        _ => None,
    }
}

fn records(request_data: &BTreeMap<String, GuardrailValue>) -> &[GuardrailValue] {
    let Some(GuardrailValue::Object(metadata)) = request_data.get("metadata") else {
        panic!("the guardrail wrote no request metadata");
    };
    let Some(GuardrailValue::Array(entries)) =
        metadata.get("standard_logging_guardrail_information")
    else {
        panic!("the guardrail wrote no guardrail record");
    };
    entries
}

// every apply_guardrail call writes exactly one record, as LiteLLM's finally block does
fn record(request_data: &BTreeMap<String, GuardrailValue>) -> &GuardrailValue {
    let entries = records(request_data);
    assert_eq!(entries.len(), 1);
    &entries[0]
}

fn detections(record: &GuardrailValue) -> &[GuardrailValue] {
    let Some(GuardrailValue::Array(found)) = field(record, &["guardrail_response"]) else {
        panic!("the record has no detection array");
    };
    found
}

fn masked_count(record: &GuardrailValue, name: &str) -> Option<i64> {
    int_field(record, &["masked_entity_count", name])
}

fn tool_arguments(inputs: &GuardrailApiInputs) -> &str {
    let Some(GuardrailValue::Object(call)) =
        inputs.tool_calls.as_deref().and_then(|calls| calls.first())
    else {
        panic!("the tool call is missing");
    };
    let Some(GuardrailValue::Object(function)) = call.get("function") else {
        panic!("the tool call has no function");
    };
    let Some(GuardrailValue::Text(arguments)) = function.get("arguments") else {
        panic!("the tool call has no arguments");
    };
    arguments
}

#[test]
fn the_defaults_match_the_litellm_class_constants() {
    let guardrail = built(args("content-filter"));
    assert_eq!(GUARDRAIL_PROVIDER, "litellm_content_filter");
    assert_eq!(guardrail.guardrail_provider, GUARDRAIL_PROVIDER);
    assert_eq!(PATTERN_REDACTION_FORMAT, "[{pattern_name}_REDACTED]");
    assert_eq!(guardrail.pattern_redaction_format, PATTERN_REDACTION_FORMAT);
    assert_eq!(KEYWORD_REDACTION_TAG, "[KEYWORD_REDACTED]");
    assert_eq!(guardrail.keyword_redaction_tag, KEYWORD_REDACTION_TAG);
    assert_eq!(guardrail.severity_threshold, "medium");
    assert_eq!(guardrail.config_guardrail_id, None);
    assert_eq!(guardrail.config_policy_template, None);
    assert_eq!(guardrail.image_model, None);
    assert!(guardrail.competitor_intent_checker.is_none());
    assert!(guardrail.compiled_patterns.is_empty());
    assert!(guardrail.blocked_words.is_empty());
    assert!(guardrail.loaded_categories.loaded_categories.is_empty());
    assert_eq!(guardrail.base.settings, expected_settings());
}

#[test]
fn the_base_settings_hold_the_configured_values() {
    let mut arguments = args("content-filter");
    arguments.default_on = true;
    arguments.end_session_after_n_fails = Some(3);
    arguments.on_violation = Some(OnViolation::Block);
    arguments.realtime_violation_message = Some("stopped".to_owned());
    let guardrail = built(arguments);
    let mut expected = expected_settings();
    expected.default_on = true;
    expected.end_session_after_n_fails = Some(3);
    expected.on_violation = Some("block".to_owned());
    expected.realtime_violation_message = Some("stopped".to_owned());
    assert_eq!(guardrail.base.settings, expected);
}

#[test]
fn an_empty_event_hook_takes_pre_call_and_a_configured_one_is_kept() {
    let mut empty_string = args("content-filter");
    empty_string.event_hook = LitellmParamsMode::One(String::new());
    assert_eq!(
        built(empty_string).base.settings.event_hook,
        Some(LitellmParamsMode::One("pre_call".to_owned()))
    );
    let mut empty_list = args("content-filter");
    empty_list.event_hook = LitellmParamsMode::Many(Vec::new());
    assert_eq!(
        built(empty_list).base.settings.event_hook,
        Some(LitellmParamsMode::One("pre_call".to_owned()))
    );
    let mut configured = args("content-filter");
    configured.event_hook = LitellmParamsMode::Many(vec!["post_call".to_owned()]);
    assert_eq!(
        built(configured).base.settings.event_hook,
        Some(LitellmParamsMode::Many(vec!["post_call".to_owned()]))
    );
}

#[test]
fn the_hooks_and_the_config_model_match_litellm() {
    let guardrail = built(args("content-filter"));
    assert_eq!(
        guardrail.get_supported_event_hooks(),
        Some(SUPPORTED_EVENT_HOOKS.to_vec())
    );
    let expected = GuardrailValue::Object(BTreeMap::from([
        (
            "model".to_owned(),
            GuardrailValue::Text("LitellmContentFilterGuardrailConfigModel".to_owned()),
        ),
        (
            "ui_friendly_name".to_owned(),
            GuardrailValue::Text("LiteLLM Content Filter".to_owned()),
        ),
    ]));
    assert_eq!(guardrail.get_config_model(), Some(expected));
}

#[test]
fn the_competitor_type_builds_the_matching_checker() {
    let mut generic_arguments = args("content-filter");
    generic_arguments.competitor_intent_config = Some(competitor(
        r#"{"competitor_intent_type": "generic", "competitors": ["globex"]}"#,
    ));
    let generic = built(generic_arguments);
    assert!(matches!(
        generic.competitor_intent_checker.as_ref(),
        Some(CompetitorIntentChecker::Generic(_))
    ));
    assert_eq!(
        intent(&generic, "globex is better"),
        Some("other".to_owned())
    );
    let mut airline_arguments = args("content-filter");
    airline_arguments.competitor_intent_config = Some(competitor(r#"{"competitors": ["globex"]}"#));
    let airline = built(airline_arguments);
    assert!(matches!(
        airline.competitor_intent_checker.as_ref(),
        Some(CompetitorIntentChecker::Airline(_))
    ));
    assert_eq!(
        intent(&airline, "globex is better"),
        Some("competitor_comparison".to_owned())
    );
}

#[test]
fn a_failed_competitor_init_logs_a_warning_and_leaves_no_checker() {
    let (_subscriber, captured) = capture();
    let mut arguments = args("content-filter");
    arguments.competitor_intent_config = Some(competitor(
        r#"{"competitors": ["alpha", "beta"], "competitor_aliases": {"alpha": ["delta"], "beta": ["delta"]}}"#,
    ));
    let guardrail = built(arguments);
    assert!(guardrail.competitor_intent_checker.is_none());
    assert!(
        captured
            .text()
            .contains("ContentFilterGuardrail: failed to init competitor intent checker: ")
    );
}

#[test]
fn during_call_with_a_mask_pattern_logs_the_warning() {
    let (_subscriber, captured) = capture();
    let mut arguments = args("content-filter");
    arguments.event_hook = LitellmParamsMode::One("during_call".to_owned());
    arguments.patterns = Some(vec![regex_pattern(
        "email",
        r"[a-z]+@example\.com",
        ContentFilterAction::Mask,
    )]);
    let _guardrail = built(arguments);
    assert!(captured.text().contains(
        "ContentFilterGuardrail 'content-filter': 'during_call' mode with 'MASK' action is unstable due to race conditions. Use 'pre_call' mode for reliable request masking."
    ));
}

#[test]
fn other_events_and_actions_do_not_log_the_warning() {
    let (first_subscriber, block_capture) = capture();
    let mut block_arguments = args("content-filter");
    block_arguments.event_hook = LitellmParamsMode::One("during_call".to_owned());
    block_arguments.patterns = Some(vec![regex_pattern(
        "email",
        r"[a-z]+@example\.com",
        ContentFilterAction::Block,
    )]);
    let _block_guardrail = built(block_arguments);
    let (second_subscriber, mask_capture) = capture();
    let mut mask_arguments = args("content-filter");
    mask_arguments.patterns = Some(vec![regex_pattern(
        "email",
        r"[a-z]+@example\.com",
        ContentFilterAction::Mask,
    )]);
    let _mask_guardrail = built(mask_arguments);
    assert!(
        !block_capture
            .text()
            .contains("is unstable due to race conditions")
    );
    assert!(
        !mask_capture
            .text()
            .contains("is unstable due to race conditions")
    );
    drop(first_subscriber);
    drop(second_subscriber);
}

#[test]
fn blocked_words_are_keyed_lowercase_and_the_last_duplicate_wins() {
    let mut arguments = args("content-filter");
    arguments.blocked_words = Some(vec![
        described_word("Secret", ContentFilterAction::Mask, "first"),
        described_word("SECRET", ContentFilterAction::Block, "second"),
        blocked_word("Other", ContentFilterAction::Mask),
    ]);
    let guardrail = built(arguments);
    let keys = guardrail.blocked_words.keys().cloned().collect::<Vec<_>>();
    assert_eq!(keys, vec!["secret".to_owned(), "other".to_owned()]);
    assert_eq!(
        guardrail.blocked_words["secret"],
        (ContentFilterAction::Block, Some("second".to_owned()))
    );
    assert_eq!(
        guardrail.blocked_words["other"],
        (ContentFilterAction::Mask, None)
    );
}

#[test]
fn a_blocked_words_file_is_merged_last_and_an_empty_path_is_skipped() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("words.yaml");
    std::fs::write(
        &path,
        "blocked_words:\n  - keyword: \"Secret\"\n    action: \"BLOCK\"\n    description: \"from file\"\n  - keyword: \"FromFile\"\n    action: \"MASK\"\n",
    )
    .unwrap();
    let mut arguments = args("content-filter");
    arguments.blocked_words = Some(vec![described_word(
        "secret",
        ContentFilterAction::Mask,
        "inline",
    )]);
    arguments.blocked_words_file = Some(path.to_str().unwrap().to_owned());
    let guardrail = built(arguments);
    assert_eq!(
        guardrail.blocked_words["secret"],
        (ContentFilterAction::Block, Some("from file".to_owned()))
    );
    assert_eq!(
        guardrail.blocked_words["fromfile"],
        (ContentFilterAction::Mask, None)
    );
    assert_eq!(guardrail.blocked_words.len(), 2);
    let mut skipped = args("content-filter");
    skipped.blocked_words_file = Some(String::new());
    assert!(built(skipped).blocked_words.is_empty());
}

#[test]
fn a_missing_blocked_words_file_fails_the_build() {
    let directory = tempfile::tempdir().unwrap();
    let mut arguments = args("content-filter");
    arguments.blocked_words_file = Some(
        directory
            .path()
            .join("missing.yaml")
            .to_str()
            .unwrap()
            .to_owned(),
    );
    let Err(error) = built_result(arguments) else {
        panic!("a missing blocked words file must fail the build");
    };
    assert!(matches!(
        error,
        ContentFilterBuildError::BlockedWords(WordsError::Read { .. })
    ));
}

#[test]
fn an_unknown_pattern_type_fails_the_build() {
    let mut arguments = args("content-filter");
    arguments.patterns = Some(vec![ContentFilterPattern {
        pattern_type: "bogus".to_owned(),
        pattern_name: None,
        pattern: None,
        name: None,
        action: ContentFilterAction::Block,
    }]);
    let Err(error) = built_result(arguments) else {
        panic!("an unknown pattern type must fail the build");
    };
    assert!(matches!(
        error,
        ContentFilterBuildError::Pattern(PatternError::UnknownType(_))
    ));
}

#[test]
fn the_categories_load_from_the_bundled_assets_and_supply_the_policy_template() {
    let mut arguments = args("content-filter");
    arguments.categories = Some(vec![category("bias_gender")]);
    let guardrail = built(arguments);
    assert!(
        guardrail
            .loaded_categories
            .loaded_categories
            .contains_key("bias_gender")
    );
    assert_eq!(
        guardrail.policy_template(),
        Some(
            "Detects gender-based discriminatory language, stereotypes, and biased content"
                .to_owned()
        )
    );
}

#[test]
fn a_configured_policy_template_wins_over_the_categories() {
    let mut arguments = args("content-filter");
    arguments.categories = Some(vec![category("bias_gender")]);
    arguments.policy_template = Some("our policy".to_owned());
    let guardrail = built(arguments);
    assert_eq!(guardrail.policy_template(), Some("our policy".to_owned()));
}

#[tokio::test]
async fn a_mask_pattern_masks_every_text_and_counts_the_masked_entities() {
    let mut arguments = args("content-filter");
    arguments.patterns = Some(vec![
        regex_pattern("email", r"[a-z]+@example\.com", ContentFilterAction::Mask),
        regex_pattern("phone", r"\d{3}-\d{4}", ContentFilterAction::Mask),
    ]);
    let guardrail = built(arguments);
    let run = applied(
        &guardrail,
        texts_inputs(&["a@example.com", "b@example.com"]),
        GuardrailInputType::Request,
    )
    .await;
    let inputs = run.result.unwrap();
    assert_eq!(
        inputs.texts,
        Some(vec![
            "[email_REDACTED]".to_owned(),
            "[email_REDACTED]".to_owned()
        ])
    );
    let entry = record(&run.request_data);
    assert_eq!(text_field(entry, &["guardrail_status"]), Some("success"));
    assert_eq!(
        text_field(entry, &["guardrail_provider"]),
        Some("litellm_content_filter")
    );
    assert_eq!(text_field(entry, &["guardrail_mode"]), Some("pre_call"));
    assert_eq!(text_field(entry, &["guardrail_id"]), Some("content-filter"));
    assert_eq!(int_field(entry, &["patterns_checked"]), Some(2));
    assert_eq!(masked_count(entry, "email"), Some(2));
    assert_eq!(masked_count(entry, "phone"), None);
    let found = detections(entry);
    assert_eq!(found.len(), 2);
    assert_eq!(text_field(&found[0], &["type"]), Some("pattern"));
    assert_eq!(text_field(&found[0], &["pattern_name"]), Some("email"));
    let start = decimal_field(entry, &["start_time"]).unwrap();
    assert!(start > 1_700_000_000.0_f64);
    assert!(decimal_field(entry, &["duration"]).unwrap() > 0.0_f64);
}

#[tokio::test]
async fn absent_texts_are_written_back_as_an_empty_list() {
    let guardrail = built(args("content-filter"));
    let run = applied(
        &guardrail,
        GuardrailApiInputs::default(),
        GuardrailInputType::Request,
    )
    .await;
    let inputs = run.result.unwrap();
    assert_eq!(inputs.texts, Some(Vec::new()));
}

#[tokio::test]
async fn a_block_returns_the_400_body_and_logs_guardrail_intervened_once() {
    let mut arguments = args("content-filter");
    arguments.blocked_words = Some(vec![blocked_word("forbidden", ContentFilterAction::Block)]);
    let guardrail = built(arguments);
    let run = applied(
        &guardrail,
        texts_inputs(&["this is forbidden"]),
        GuardrailInputType::Request,
    )
    .await;
    let failure = run.result.unwrap_err();
    let GuardrailFailure::HttpObject { status, detail } = failure else {
        panic!("a blocked word must raise the 400 object");
    };
    assert_eq!(status, 400_u16);
    assert_eq!(
        detail.get("error"),
        Some(&GuardrailValue::Text(
            "Content blocked: keyword 'forbidden' detected".to_owned()
        ))
    );
    assert_eq!(
        detail.get("keyword"),
        Some(&GuardrailValue::Text("forbidden".to_owned()))
    );
    assert_eq!(detail.get("description"), Some(&GuardrailValue::Null));
    let entry = record(&run.request_data);
    assert_eq!(
        text_field(entry, &["guardrail_status"]),
        Some("guardrail_intervened")
    );
    let found = detections(entry);
    assert_eq!(found.len(), 1);
    assert_eq!(text_field(&found[0], &["type"]), Some("blocked_word"));
    assert_eq!(text_field(&found[0], &["keyword"]), Some("forbidden"));
    assert_eq!(text_field(&found[0], &["action"]), Some("BLOCK"));
}

#[tokio::test]
async fn a_blocked_text_that_follows_a_masked_one_records_no_masked_counts() {
    let mut arguments = args("content-filter");
    arguments.blocked_words = Some(vec![
        blocked_word("secret", ContentFilterAction::Mask),
        blocked_word("forbidden", ContentFilterAction::Block),
    ]);
    let guardrail = built(arguments);
    let run = applied(
        &guardrail,
        texts_inputs(&["my secret", "forbidden"]),
        GuardrailInputType::Request,
    )
    .await;
    assert!(matches!(
        run.result.unwrap_err(),
        GuardrailFailure::HttpObject { .. }
    ));
    let entry = record(&run.request_data);
    assert_eq!(
        field(entry, &["masked_entity_count"]),
        Some(&GuardrailValue::Object(BTreeMap::new()))
    );
    assert_eq!(detections(entry).len(), 2);
}

#[tokio::test]
async fn images_texts_and_tool_calls_are_scanned_in_litellm_order() {
    let mut arguments = args("content-filter");
    arguments.image_model = Some("image-model".to_owned());
    arguments.blocked_words = Some(vec![
        blocked_word("alpha", ContentFilterAction::Mask),
        blocked_word("beta", ContentFilterAction::Mask),
        blocked_word("gamma", ContentFilterAction::Mask),
    ]);
    let guardrail = built_with_router(
        arguments,
        TestRouter {
            description: Some("alpha image".to_owned()),
        },
    );
    let mut inputs = texts_inputs(&["beta text"]);
    inputs.images = Some(vec!["https://example.test/cat.png".to_owned()]);
    inputs.tool_calls = Some(vec![tool_call(r#"{"query":"gamma"}"#)]);
    let run = applied(&guardrail, inputs, GuardrailInputType::Request).await;
    let entry = record(&run.request_data);
    let found = detections(entry);
    assert_eq!(found.len(), 3);
    assert_eq!(text_field(&found[0], &["keyword"]), Some("alpha"));
    assert_eq!(text_field(&found[1], &["keyword"]), Some("beta"));
    assert_eq!(text_field(&found[2], &["keyword"]), Some("gamma"));
    assert_eq!(masked_count(entry, "blocked_word"), Some(3));
}

#[tokio::test]
async fn tool_call_arguments_are_scanned_on_a_request_and_on_a_response() {
    let mut arguments = args("content-filter");
    arguments.blocked_words = Some(vec![blocked_word("secret", ContentFilterAction::Mask)]);
    let guardrail = built(arguments);
    for input_type in [GuardrailInputType::Request, GuardrailInputType::Response] {
        let mut inputs = GuardrailApiInputs::default();
        inputs.tool_calls = Some(vec![tool_call(r#"{"query":"secret"}"#)]);
        let run = applied(&guardrail, inputs, input_type).await;
        assert_eq!(
            tool_arguments(&run.result.unwrap()),
            r#"{"query":"[KEYWORD_REDACTED]"}"#
        );
    }
}

#[tokio::test]
async fn mcp_arguments_are_scanned_only_for_a_request() {
    let mut arguments = args("content-filter");
    arguments.event_hook = LitellmParamsMode::One("pre_mcp_call".to_owned());
    arguments.blocked_words = Some(vec![blocked_word("secret", ContentFilterAction::Mask)]);
    let guardrail = built(arguments);
    let request = applied_with(
        &guardrail,
        GuardrailApiInputs::default(),
        mcp_request_data(),
        GuardrailInputType::Request,
    )
    .await;
    let mcp = request.request_data.get("mcp_arguments").unwrap();
    assert_eq!(
        field(mcp, &["query"]),
        Some(&GuardrailValue::Text("[KEYWORD_REDACTED] stuff".to_owned()))
    );
    assert!(request.request_data.contains_key("modified_arguments"));
    let response = applied_with(
        &guardrail,
        GuardrailApiInputs::default(),
        mcp_request_data(),
        GuardrailInputType::Response,
    )
    .await;
    assert!(!response.request_data.contains_key("modified_arguments"));
    assert_eq!(
        field(
            response.request_data.get("mcp_arguments").unwrap(),
            &["query"]
        ),
        Some(&GuardrailValue::Text("secret stuff".to_owned()))
    );
}

#[tokio::test]
async fn a_competitor_intent_that_the_policy_allows_still_logs_a_detection() {
    let mut arguments = args("content-filter");
    arguments.competitor_intent_config = Some(competitor(r#"{"competitors": ["globex"]}"#));
    let guardrail = built(arguments);
    let run = applied(
        &guardrail,
        texts_inputs(&["globex is better"]),
        GuardrailInputType::Request,
    )
    .await;
    let inputs = run.result.unwrap();
    assert_eq!(inputs.texts, Some(vec!["globex is better".to_owned()]));
    let entry = record(&run.request_data);
    assert_eq!(text_field(entry, &["guardrail_status"]), Some("success"));
    let found = detections(entry);
    assert_eq!(found.len(), 1);
    assert_eq!(text_field(&found[0], &["type"]), Some("competitor_intent"));
    assert_eq!(
        text_field(&found[0], &["intent"]),
        Some("competitor_comparison")
    );
    assert_eq!(text_field(&found[0], &["action_hint"]), Some("allow"));
    assert_eq!(decimal_field(&found[0], &["confidence"]), Some(0.75_f64));
}

#[tokio::test]
async fn a_reframe_policy_fails_the_guardrail_and_logs_failed_to_respond() {
    let mut arguments = args("content-filter");
    arguments.competitor_intent_config = Some(competitor(
        r#"{"competitors": ["globex"], "policy": {"competitor_comparison": "reframe"}, "reframe_message_template": "Compare with our own flights."}"#,
    ));
    let guardrail = built(arguments);
    let run = applied(
        &guardrail,
        texts_inputs(&["globex is better"]),
        GuardrailInputType::Request,
    )
    .await;
    let failure = run.result.unwrap_err();
    let GuardrailFailure::ModifyResponse(modify) = &failure else {
        panic!("a reframe policy must modify the response");
    };
    assert_eq!(modify.message, "Compare with our own flights.");
    let entry = record(&run.request_data);
    assert_eq!(
        text_field(entry, &["guardrail_status"]),
        Some("guardrail_failed_to_respond")
    );
    assert_eq!(
        text_field(entry, &["guardrail_response"]),
        Some("Compare with our own flights.")
    );
}

#[tokio::test]
async fn a_refuse_policy_returns_the_configured_refuse_message() {
    let mut arguments = args("content-filter");
    arguments.competitor_intent_config = Some(competitor(
        r#"{"competitors": ["globex"], "policy": {"competitor_comparison": "refuse"}, "refuse_message_template": "No competitor talk."}"#,
    ));
    let guardrail = built(arguments);
    let run = applied(
        &guardrail,
        texts_inputs(&["globex is better"]),
        GuardrailInputType::Request,
    )
    .await;
    let GuardrailFailure::HttpObject { status, detail } = run.result.unwrap_err() else {
        panic!("a refuse policy must raise the 400 object");
    };
    assert_eq!(status, 400_u16);
    assert_eq!(
        detail.get("error"),
        Some(&GuardrailValue::Text("No competitor talk.".to_owned()))
    );
    assert_eq!(
        detail.get("intent"),
        Some(&GuardrailValue::Text("competitor_comparison".to_owned()))
    );
    let entry = record(&run.request_data);
    assert_eq!(
        text_field(entry, &["guardrail_status"]),
        Some("guardrail_intervened")
    );
}

#[tokio::test]
async fn a_failing_image_description_logs_guardrail_failed_to_respond() {
    let mut arguments = args("content-filter");
    arguments.image_model = Some("image-model".to_owned());
    let guardrail = built_with_router(arguments, TestRouter { description: None });
    let mut inputs = GuardrailApiInputs::default();
    inputs.images = Some(vec!["https://example.test/cat.png".to_owned()]);
    let run = applied(&guardrail, inputs, GuardrailInputType::Request).await;
    let failure = run.result.unwrap_err();
    assert_eq!(failure.to_string(), "the router is unavailable");
    let entry = record(&run.request_data);
    assert_eq!(
        text_field(entry, &["guardrail_status"]),
        Some("guardrail_failed_to_respond")
    );
    assert_eq!(
        text_field(entry, &["guardrail_response"]),
        Some("the router is unavailable")
    );
}

#[tokio::test]
async fn a_blocked_image_description_carries_the_description_in_the_400() {
    let mut arguments = args("content-filter");
    arguments.image_model = Some("image-model".to_owned());
    arguments.blocked_words = Some(vec![blocked_word("forbidden", ContentFilterAction::Block)]);
    let guardrail = built_with_router(
        arguments,
        TestRouter {
            description: Some("a forbidden scene".to_owned()),
        },
    );
    let mut inputs = GuardrailApiInputs::default();
    inputs.images = Some(vec!["https://example.test/cat.png".to_owned()]);
    let run = applied(&guardrail, inputs, GuardrailInputType::Request).await;
    let GuardrailFailure::HttpObject { status, detail } = run.result.unwrap_err() else {
        panic!("a blocked image description must raise the 400 object");
    };
    assert_eq!(status, 400_u16);
    assert_eq!(
        detail.get("error"),
        Some(&GuardrailValue::Text(
            "Content blocked: keyword 'forbidden' detected (Image description): a forbidden scene"
                .to_owned()
        ))
    );
}
