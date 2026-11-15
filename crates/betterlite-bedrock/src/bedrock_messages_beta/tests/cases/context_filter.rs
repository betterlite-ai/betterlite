use std::collections::BTreeSet;

use betterlite_prompts::arguments::json::Object;

use crate::bedrock_messages_beta::context_management::filter_context_management;
use crate::bedrock_messages_beta::tests::support::{capture_logs, obj};

const MIXED_EDITS: &str = r#"{
  "context_management": {
    "keep": true,
    "edits": [
      {"type": "clear_thinking_20251015"},
      {"type": "compact_20260112", "trigger": {"tokens": 1000}, "instructions": "sum"},
      {"type": "clear_tool_uses_20250919", "keep": 2},
      "not-a-dict",
      {"type": 5}
    ]
  },
  "max_tokens": 8
}"#;

fn filtered(request: &str) -> (Object, BTreeSet<String>) {
    let mut request = obj(request);
    let mut betas = BTreeSet::new();
    betas.insert("effort-2025-11-24".to_owned());
    filter_context_management(&mut request, &mut betas);
    (request, betas)
}

pub(crate) fn run() {
    let (_guard, logs) = capture_logs();

    // a non-object context_management is left for someone else
    let untouched = r#"{"context_management": "text", "max_tokens": 8}"#;
    assert_eq!(filtered(untouched).0, obj(untouched));

    // a dict without a usable edits list loses the whole field
    assert_eq!(
        filtered(r#"{"context_management": {}, "max_tokens": 8}"#).0,
        obj(r#"{"max_tokens": 8}"#)
    );
    assert_eq!(
        filtered(r#"{"context_management": {"edits": "x"}, "max_tokens": 8}"#).0,
        obj(r#"{"max_tokens": 8}"#)
    );
    assert_eq!(
        filtered(r#"{"context_management": {"edits": []}, "max_tokens": 8}"#).0,
        obj(r#"{"max_tokens": 8}"#)
    );
    assert!(
        filtered(
            r#"{"context_management": {"edits": [{"type": "clear_thinking_20251015"}, "x", {"type": 5}]}}"#
        )
        .0
        .is_empty()
    );

    // retained edits keep their order and every field, and their other fields survive
    let (request, betas) = filtered(MIXED_EDITS);
    assert_eq!(
        request,
        obj(r#"{
  "context_management": {
    "keep": true,
    "edits": [
      {"type": "compact_20260112", "trigger": {"tokens": 1000}, "instructions": "sum"},
      {"type": "clear_tool_uses_20250919", "keep": 2}
    ]
  },
  "max_tokens": 8
}"#)
    );
    assert_eq!(
        betas,
        BTreeSet::from([
            "compact-2026-01-12".to_owned(),
            "context-management-2025-06-27".to_owned(),
            "effort-2025-11-24".to_owned(),
        ])
    );

    // an absent field is not the filter's business either
    assert_eq!(
        filtered(r#"{"max_tokens": 8}"#).0,
        obj(r#"{"max_tokens": 8}"#)
    );
    assert!(logs.warnings().is_empty());
}
