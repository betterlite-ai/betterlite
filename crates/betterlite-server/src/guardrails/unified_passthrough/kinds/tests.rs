#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::routes::CallType;
use betterlite_guardrails::llm_passthrough_dispatch::{
    PASSTHROUGH_TRANSLATION_KINDS, PassthroughTranslationKind,
};
use rstest::rstest;

use super::{PASSTHROUGH_TABLE, PassthroughKind, passthrough_provider_list};
use crate::guardrails::bedrock_passthrough::text_paths::SkipFlags;
use crate::guardrails::unified_passthrough::bedrock::BedrockPassthroughTranslation;
use crate::guardrails::unified_passthrough::tests::{FakeGuardrail, TestHook};

#[test]
fn the_provider_list_holds_the_bedrock_adapter_under_its_key() {
    let metadata = BTreeMap::from([(
        "user_api_key_user_id".to_owned(),
        GuardrailValue::Text("u1".to_owned()),
    )]);
    let bedrock = BedrockPassthroughTranslation::<FakeGuardrail, TestHook>::new(
        TestHook::echoing(),
        SkipFlags {
            system: true,
            tool: false,
        },
        metadata.clone(),
    );
    let providers = passthrough_provider_list(bedrock);
    assert_eq!(providers.len(), 1);
    let (key, adapter) = providers.first().expect("one provider");
    assert_eq!(key.as_str(), "bedrock");
    assert!(adapter.skip_system);
    assert!(!adapter.skip_tool);
    assert_eq!(adapter.user_metadata, metadata);
}

#[rstest]
#[case(0)]
#[case(1)]
fn the_table_mirrors_the_b3574_rows(#[case] index: usize) {
    let (call_type, kind) = PASSTHROUGH_TABLE.get(index).copied().expect("two rows");
    let (expected_call, expected_kind) = PASSTHROUGH_TRANSLATION_KINDS
        .get(index)
        .copied()
        .expect("two rows");
    assert_eq!(call_type, expected_call);
    let expected = match expected_kind {
        PassthroughTranslationKind::Endpoint => PassthroughKind::Endpoint,
        PassthroughTranslationKind::LlmRoute => PassthroughKind::LlmRoute,
    };
    assert_eq!(kind, expected);
}

#[test]
fn the_table_has_no_row_for_the_sync_llm_route() {
    assert_eq!(PASSTHROUGH_TABLE.len(), 2);
    assert_eq!(PASSTHROUGH_TABLE.len(), PASSTHROUGH_TRANSLATION_KINDS.len());
    assert!(
        !PASSTHROUGH_TABLE
            .iter()
            .any(|(call_type, _kind)| *call_type == CallType::LlmPassthroughRoute)
    );
}
