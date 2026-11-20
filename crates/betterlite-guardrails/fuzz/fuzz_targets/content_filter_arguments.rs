#![no_main]

use betterlite_config::sections::content_category::loading::CategoryLoadResult;
use betterlite_config::sections::content_filter_inputs::DEFAULT_RECURSE_DEPTH;
use betterlite_config::sections::content_filter_records::{
    ContentFilterAction, ContentFilterPattern,
};
use betterlite_guardrails::content_filter_actions::ContentFilterSource;
use betterlite_guardrails::content_filter_inputs::arguments::{
    ArgumentFilter, filter_tool_call_arguments,
};
use betterlite_guardrails::pattern_matching::compile;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|arguments: &str| {
    let categories = CategoryLoadResult::default();
    let config = ContentFilterPattern {
        pattern_type: "regex".to_owned(),
        pattern_name: None,
        pattern: Some("\\d".to_owned()),
        name: Some("digit".to_owned()),
        action: ContentFilterAction::Mask,
    };
    let Ok(pattern) = compile(&config) else {
        return;
    };
    let patterns = [pattern];
    let source = ContentFilterSource {
        categories: &categories,
        patterns: &patterns,
        blocked_words: &[],
        pattern_redaction_format: "[{pattern_name}_REDACTED]",
        keyword_redaction_tag: "[KEYWORD_REDACTED]",
    };
    let filter = ArgumentFilter {
        source: &source,
        max_depth: DEFAULT_RECURSE_DEPTH,
    };
    let mut detections = Vec::new();
    drop(filter_tool_call_arguments(
        &filter,
        arguments,
        &mut detections,
    ));
});
