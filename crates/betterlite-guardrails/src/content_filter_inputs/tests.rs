#![cfg(test)]

mod arguments;
mod competitor;
mod images;
mod mcp;

use std::collections::BTreeMap;
use std::io;
use std::io::Write;
use std::sync::{Arc, Mutex};

use tracing::subscriber::with_default;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::content_category::loading::CategoryLoadResult;
use betterlite_config::sections::content_filter_records::{
    BlockedWord, ContentFilterAction, ContentFilterDetection, ContentFilterPattern,
};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;

use crate::content_filter_actions::ContentFilterSource;
use crate::content_filter_inputs::arguments::{ArgumentFilter, filter_argument_value};
use crate::content_filter_inputs::error::ArgumentFilterError;
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};
use crate::pattern_matching::{CompiledPattern, compile};

const PATTERN_FORMAT: &str = "[{pattern_name}_REDACTED]";
const KEYWORD_TAG: &str = "[KEYWORD_REDACTED]";

pub(super) struct Fixture {
    categories: CategoryLoadResult,
    patterns: Vec<CompiledPattern>,
    blocked_words: Vec<BlockedWord>,
    pattern_redaction_format: String,
    keyword_redaction_tag: String,
}

impl Fixture {
    pub(super) fn source(&self) -> ContentFilterSource<'_> {
        ContentFilterSource {
            categories: &self.categories,
            patterns: &self.patterns,
            blocked_words: &self.blocked_words,
            pattern_redaction_format: &self.pattern_redaction_format,
            keyword_redaction_tag: &self.keyword_redaction_tag,
        }
    }

    pub(super) fn empty() -> Self {
        Self::from_parts(Vec::new(), Vec::new())
    }

    pub(super) fn with_patterns(patterns: Vec<CompiledPattern>) -> Self {
        Self::from_parts(patterns, Vec::new())
    }

    fn from_parts(patterns: Vec<CompiledPattern>, blocked_words: Vec<BlockedWord>) -> Self {
        Self {
            categories: CategoryLoadResult::default(),
            patterns,
            blocked_words,
            pattern_redaction_format: PATTERN_FORMAT.to_owned(),
            keyword_redaction_tag: KEYWORD_TAG.to_owned(),
        }
    }
}

pub(super) fn regex_pattern(
    name: &str,
    source: &str,
    action: ContentFilterAction,
) -> CompiledPattern {
    let config = ContentFilterPattern {
        pattern_type: "regex".to_owned(),
        pattern_name: None,
        pattern: Some(source.to_owned()),
        name: Some(name.to_owned()),
        action,
    };
    compile(&config).expect("the test regex pattern compiles")
}

pub(super) fn pattern_names(detections: &[ContentFilterDetection]) -> Vec<&str> {
    detections
        .iter()
        .map(|detection| match detection {
            ContentFilterDetection::Pattern(record) => record.pattern_name.as_str(),
            ContentFilterDetection::BlockedWord(record) => record.keyword.as_str(),
            ContentFilterDetection::CategoryKeyword(record) => record.keyword.as_str(),
            ContentFilterDetection::CompetitorIntent(record) => record.intent.as_str(),
        })
        .collect()
}

pub(super) fn base(event_hook: Option<LitellmParamsMode>) -> GuardrailBase {
    let settings = GuardrailSettings {
        guardrail_name: Some("test-content-filter".to_owned()),
        supported_event_hooks: None,
        event_hook,
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
        switches: GuardrailSwitches::default(),
    };
    GuardrailBase::new(settings, true).expect("the test guardrail settings are valid")
}

pub(super) fn pre_mcp_call_hook() -> LitellmParamsMode {
    LitellmParamsMode::One("pre_mcp_call".to_owned())
}

pub(super) fn nested_array(depth: i64) -> GuardrailValue {
    let mut value = GuardrailValue::Text("leaf".to_owned());
    for _level in 0..depth {
        value = GuardrailValue::Array(vec![value]);
    }
    value
}

fn load_depth(environment: &[(&str, &str)]) -> i64 {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let config = ConfigLoader { lookup }
        .load("")
        .expect("the content filter input config loads");
    config.sections.content_filter_inputs.max_recurse_depth
}

fn check_depth(depth: i64) {
    let fixture = Fixture::empty();
    let source = fixture.source();
    let filter = ArgumentFilter {
        source: &source,
        max_depth: depth,
    };
    let mut detections = Vec::new();
    filter_argument_value(
        &filter,
        &nested_array(depth),
        &mut detections,
        "tool call",
        0,
    )
    .expect("a value at the depth filters");
    let error = filter_argument_value(
        &filter,
        &nested_array(depth.saturating_add(1)),
        &mut detections,
        "tool call",
        0,
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Content blocked: tool call arguments exceed the maximum nesting depth"
    );
}

#[test]
fn depth_limit_and_config() {
    let default_depth = load_depth(&[]);
    assert_eq!(default_depth, 100);
    let configured = load_depth(&[("DEFAULT_MAX_RECURSE_DEPTH", "2")]);
    assert_eq!(configured, 2);
    check_depth(default_depth);
    check_depth(configured);
}

#[test]
fn depth_recursion_covers_objects_and_lists() {
    let fixture = Fixture::empty();
    let source = fixture.source();
    let filter = ArgumentFilter {
        source: &source,
        max_depth: 2,
    };
    let mut detections = Vec::new();
    let object = GuardrailValue::Object(BTreeMap::from([(
        "outer".to_owned(),
        GuardrailValue::Array(vec![GuardrailValue::Text("leaf".to_owned())]),
    )]));
    filter_argument_value(&filter, &object, &mut detections, "tool call", 0)
        .expect("an object holding a list at the limit filters");
    let deeper = nested_array(3);
    let error =
        filter_argument_value(&filter, &deeper, &mut detections, "tool call", 0).unwrap_err();
    assert!(matches!(error, ArgumentFilterError::Depth { .. }));
}

#[derive(Clone, Default)]
pub(super) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("poisoned log buffer"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn with_logs<Outcome, Run>(run: Run) -> (Outcome, String)
where
    Run: FnOnce() -> Outcome,
{
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let outcome = with_default(subscriber, run);
    let buffer = captured.0.lock().expect("log buffer").clone();
    (
        outcome,
        String::from_utf8(buffer).expect("utf-8 log output"),
    )
}
