#![cfg(test)]

use super::{CountedBreakpoint, CountedPromptCachePlan};

#[test]
fn breakpoint_defaults_content_fingerprint_and_its_lookback_list() {
    let json = r#"{
        "fingerprint": "fp",
        "ttl_seconds": 300,
        "prefix_tokens": 10,
        "lookback_fingerprints": ["fp"]
    }"#;
    let breakpoint: CountedBreakpoint = serde_json::from_str(json).expect("minimal breakpoint");
    assert_eq!(breakpoint.content_fingerprint, "");
    assert!(breakpoint.lookback_content_fingerprints.is_empty());
}

#[test]
fn plan_rejects_an_unknown_field() {
    let json = r#"{"total_tokens": 1, "breakpoints": [], "extra": true}"#;
    let result: Result<CountedPromptCachePlan, _> = serde_json::from_str(json);
    result.unwrap_err();
}
