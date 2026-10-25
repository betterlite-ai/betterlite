#![cfg(test)]
use rstest::rstest;

use crate::errors::HarnessError;
use crate::options::ReasoningEffort;
use crate::types::{FileChangeKind, Harness, PermissionMode, StopReason, Usage, require_harness};

#[rstest]
#[case::claude_code(Harness::ClaudeCode, "claude_code", "CLAUDE_CODE")]
#[case::codex(Harness::Codex, "codex", "CODEX")]
#[case::opencode(Harness::OpenCode, "opencode", "OPENCODE")]
#[case::deepagents(Harness::DeepAgents, "deepagents", "DEEPAGENTS")]
fn harness_keeps_litellm_value_and_name(
    #[case] harness: Harness,
    #[case] value: &str,
    #[case] name: &str,
) {
    assert_eq!(
        serde_json::to_string(&harness).unwrap(),
        format!("\"{value}\"")
    );
    assert_eq!(harness.name(), name);
}

#[rstest]
#[case::value("codex", Harness::Codex)]
#[case::upper_name("CLAUDE_CODE", Harness::ClaudeCode)]
#[case::dashes_and_spaces(" Claude-Code ", Harness::ClaudeCode)]
#[case::deepagents("DeepAgents", Harness::DeepAgents)]
fn require_harness_hints_a_member(#[case] given: &str, #[case] expected: Harness) {
    let HarnessError::HarnessHint { given: kept, hint } = require_harness(given) else {
        panic!("no hint for {given}");
    };
    assert_eq!((kept.as_str(), hint), (given, expected));
}

#[test]
fn require_harness_names_the_hint_or_none() {
    let hinted = require_harness(" OpenCode").to_string();
    assert_eq!(
        hinted,
        "harness must be a Harness member, got the string \" OpenCode\"; did you mean Harness.OPENCODE?"
    );
    let error = require_harness("open-code");
    assert_eq!(
        error.to_string(),
        "harness must be a Harness member, got the string \"open-code\""
    );
}

#[test]
fn usage_defaults_to_zero_and_sums_tokens() {
    let empty = Usage::default();
    assert_eq!(
        (empty.input_tokens, empty.output_tokens, empty.calls),
        (0, 0, 0)
    );
    let counted = Usage {
        input_tokens: 2,
        output_tokens: 3,
        calls: 1,
    };
    assert_eq!(counted.total_tokens().unwrap(), 5);
    let huge = Usage {
        input_tokens: u64::MAX,
        output_tokens: 1,
        calls: 1,
    };
    assert!(
        matches!(huge.total_tokens(), Err(HarnessError::TokenOverflow { usage }) if usage == huge)
    );
}

#[rstest]
#[case::stop_done(serde_json::to_string(&StopReason::Done), "done")]
#[case::stop_max_turns(serde_json::to_string(&StopReason::MaxTurns), "max_turns")]
#[case::stop_timeout(serde_json::to_string(&StopReason::Timeout), "timeout")]
#[case::stop_cancelled(serde_json::to_string(&StopReason::Cancelled), "cancelled")]
#[case::stop_runtime_error(serde_json::to_string(&StopReason::RuntimeError), "runtime_error")]
#[case::permission_read_only(serde_json::to_string(&PermissionMode::ReadOnly), "read-only")]
#[case::permission_ask(serde_json::to_string(&PermissionMode::Ask), "ask")]
#[case::permission_edit(serde_json::to_string(&PermissionMode::Edit), "edit")]
#[case::permission_full(serde_json::to_string(&PermissionMode::Full), "full")]
#[case::file_created(serde_json::to_string(&FileChangeKind::Created), "created")]
#[case::file_modified(serde_json::to_string(&FileChangeKind::Modified), "modified")]
#[case::file_deleted(serde_json::to_string(&FileChangeKind::Deleted), "deleted")]
#[case::effort_low(serde_json::to_string(&ReasoningEffort::Low), "low")]
#[case::effort_medium(serde_json::to_string(&ReasoningEffort::Medium), "medium")]
#[case::effort_high(serde_json::to_string(&ReasoningEffort::High), "high")]
#[case::effort_xhigh(serde_json::to_string(&ReasoningEffort::Xhigh), "xhigh")]
fn literal_sets_keep_litellm_spelling(
    #[case] encoded: serde_json::Result<String>,
    #[case] value: &str,
) {
    assert_eq!(encoded.unwrap(), format!("\"{value}\""));
}
