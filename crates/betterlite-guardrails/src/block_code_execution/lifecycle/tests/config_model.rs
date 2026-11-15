use std::any::TypeId;

use betterlite_config::sections::content_filter_records::{
    BlockCodeAction, BlockCodeExecGuardrailConfig,
};
use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity as _;
use betterlite_guardrails::block_code_execution::registration::BlockCodeExecutionConfigModel;

#[test]
fn get_config_model_is_the_block_code_execution_config_model() {
    assert_eq!(
        TypeId::of::<BlockCodeExecutionConfigModel>(),
        TypeId::of::<BlockCodeExecGuardrailConfig>()
    );
}

#[test]
fn the_config_model_defaults_match_litellm() {
    let model: BlockCodeExecutionConfigModel =
        serde_json::from_str("{}").expect("empty config model");
    assert!(model.blocked_languages.is_none());
    assert_eq!(model.action, BlockCodeAction::Block);
    assert_eq!(model.confidence_threshold.0.to_bits(), 0.5_f64.to_bits());
    assert!(model.detect_execution_intent);
}

#[test]
fn the_config_model_reads_every_block_code_setting() {
    let model: BlockCodeExecutionConfigModel = serde_json::from_str(
        r#"{"blocked_languages": ["python", "bash"], "action": "mask", "confidence_threshold": 0.9, "detect_execution_intent": false}"#,
    )
    .expect("configured model");
    assert_eq!(
        model.blocked_languages,
        Some(vec!["python".to_owned(), "bash".to_owned()])
    );
    assert_eq!(model.action, BlockCodeAction::Mask);
    assert_eq!(model.confidence_threshold.0.to_bits(), 0.9_f64.to_bits());
    assert!(!model.detect_execution_intent);
}

#[test]
fn the_ui_name_is_block_code_execution() {
    assert_eq!(
        BlockCodeExecutionConfigModel::ui_friendly_name(),
        "Block Code Execution"
    );
}
