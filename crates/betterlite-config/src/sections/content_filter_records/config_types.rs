#![cfg(test)]

use super::{
    BlockCodeExecGuardrailConfig, CategoryFileData, CodeBlockActionTaken, CodeBlockDetection,
    ContentFilterAction, ContentFilterCategoryConfig, ContentFilterDetection,
    ContentFilterGuardrailConfig, HideSecretsGuardrailConfig,
};

#[test]
fn config_defaults_match_litellm() {
    let content: ContentFilterGuardrailConfig = serde_json::from_str("{}").unwrap();
    assert_eq!(content.severity_threshold, "medium");
    assert_eq!(
        content.pattern_redaction_format,
        "[{pattern_name}_REDACTED]"
    );
    assert_eq!(
        content.keyword_redaction_tag,
        "[KEYWORD_REDACTED]"
    );

    let code: BlockCodeExecGuardrailConfig = serde_json::from_str("{}").unwrap();
    assert_eq!(code.action, "block");
    assert!((code.confidence_threshold - 0.5_f64).abs() < f64::EPSILON);
    assert!(code.detect_execution_intent);
}

#[test]
fn config_records_deserialize_litellm_fields() {
    let category: ContentFilterCategoryConfig = serde_json::from_str(
        r#"{"category":"harmful_self_harm","action":"BLOCK","category_file":"custom.yaml"}"#,
    )
    .unwrap();
    assert_eq!(category.action, ContentFilterAction::Block);
    assert!(category.enabled);
    assert_eq!(category.severity_threshold, "medium");

    let secrets: HideSecretsGuardrailConfig = serde_json::from_str(
        r#"{"detect_secrets_config":{"plugins_used":[]}}"#,
    )
    .unwrap();
    assert!(secrets.detect_secrets_config.is_some());
}

#[test]
fn detection_union_and_category_file_deserialize() {
    let detection: ContentFilterDetection = serde_json::from_str(
        r#"{"type":"blocked_word","keyword":"secret","action":"MASK","description":null}"#,
    )
    .unwrap();
    assert!(matches!(detection, ContentFilterDetection::BlockedWord(_)));

    let code: CodeBlockDetection = serde_json::from_str(
        r#"{"type":"code_block","language":"python","confidence":0.9,"action_taken":"log_only"}"#,
    )
    .unwrap();
    assert_eq!(code.action_taken, CodeBlockActionTaken::LogOnly);

    let file: CategoryFileData = serde_json::from_str(
        r#"{"category_name":"harmful","keywords":[{"keyword":"danger","severity":"high"}],"phrase_patterns":["do harm"]}"#,
    )
    .unwrap();
    assert_eq!(file.category_name.as_deref(), Some("harmful"));
    assert_eq!(file.keywords.len(), 1);
    assert_eq!(file.phrase_patterns, ["do harm"]);
}

#[test]
fn ui_names_are_stable() {
    assert_eq!(
        ContentFilterGuardrailConfig::ui_friendly_name(),
        "LiteLLM Content Filter"
    );
    assert_eq!(
        BlockCodeExecGuardrailConfig::ui_friendly_name(),
        "Block Code Execution"
    );
    assert_eq!(HideSecretsGuardrailConfig::ui_friendly_name(), "Hide Secrets");
}
