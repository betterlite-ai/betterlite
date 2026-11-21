#![cfg(test)]

use rstest::rstest;

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

#[rstest]
#[case::responses(CallType::Responses)]
#[case::aresponses(CallType::Aresponses)]
#[case::anthropic_messages(CallType::AnthropicMessages)]
fn table_only_chat_entries_are_chat_completions(#[case] call_type: CallType) {
    assert_eq!(CHAT_COMPLETIONS_CALL_TYPES, CHAT_CALL_TYPES);
    assert!(!CHAT_COMPLETIONS_CALL_TYPES.contains(&call_type));
}

#[rstest]
#[case::completion(CallType::Completion)]
#[case::acompletion(CallType::Acompletion)]
#[case::responses(CallType::Responses)]
#[case::aresponses(CallType::Aresponses)]
#[case::anthropic_messages(CallType::AnthropicMessages)]
fn table_no_call_type_repeats(#[case] call_type: CallType) {
    let all = [
        &CHAT_CALL_TYPES[..],
        &RESPONSES_CALL_TYPES[..],
        &ANTHROPIC_CALL_TYPES[..],
    ]
    .concat();
    assert_eq!(all.iter().filter(|item| **item == call_type).count(), 1);
}
