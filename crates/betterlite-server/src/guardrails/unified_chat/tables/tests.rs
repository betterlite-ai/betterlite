#![cfg(test)]

use betterlite_core::routes::CallType;

use super::{
    ANTHROPIC_CALL_TYPES, CHAT_CALL_TYPES, CHAT_COMPLETIONS_CALL_TYPES, RESPONSES_CALL_TYPES,
};

#[test]
fn table_chat_call_types_in_order() {
    assert_eq!(
        CHAT_CALL_TYPES,
        [CallType::Completion, CallType::Acompletion]
    );
}

#[test]
fn table_responses_call_types_in_order() {
    assert_eq!(
        RESPONSES_CALL_TYPES,
        [CallType::Responses, CallType::Aresponses]
    );
}

#[test]
fn table_anthropic_call_types_in_order() {
    assert_eq!(ANTHROPIC_CALL_TYPES, [CallType::AnthropicMessages]);
}

#[test]
fn table_only_chat_entries_are_chat_completions() {
    assert_eq!(CHAT_COMPLETIONS_CALL_TYPES, CHAT_CALL_TYPES);
    for call_type in RESPONSES_CALL_TYPES
        .iter()
        .chain(ANTHROPIC_CALL_TYPES.iter())
    {
        assert!(!CHAT_COMPLETIONS_CALL_TYPES.contains(call_type));
    }
}

#[test]
fn table_no_call_type_repeats() {
    let all: Vec<CallType> = CHAT_COMPLETIONS_CALL_TYPES
        .iter()
        .chain(RESPONSES_CALL_TYPES.iter())
        .chain(ANTHROPIC_CALL_TYPES.iter())
        .copied()
        .collect();
    for (index, call_type) in all.iter().enumerate() {
        assert!(!all[..index].contains(call_type));
    }
}
