#![cfg(test)]

mod apply;
mod construction;
mod masking;
mod response;
mod streaming;
mod support;

use rstest::rstest;

#[derive(Debug, Clone, Copy)]
enum Case {
    DefaultsAreApplied,
    ConfiguredValuesAreApplied,
    HookWidensForRolelessOutputCallback,
    HookWidensForRolelessOutputMaskCallback,
    HookWideningAppendsToList,
    HookWideningKeepsListWithPostCall,
    HookWideningSkipsCallbackWithRole,
    HookWideningSkipsRolelessCallbackWithoutOutput,
    HookWideningKeepsTaggedMode,
    LoggingOnlySetsEventHook,
    MissingRecognizerFile,
    InvalidRecognizerJson,
    UnreadableRecognizerFile,
    RecognizerFileIsNotAList,
    RecognizerFileIsReadFirst,
    MissingAnalyzerBase,
    MissingAnonymizerBase,
    DefaultsSupplyApiBases,
    ExplicitBaseWins,
    UnrepresentableTimeout,
    MasksStringContent,
    MasksListContent,
    SkipsOtherContent,
    ReturnsWhenMessagesAreAbsent,
    FailsWhenMessagesAreNotAList,
    SkipsLoggingOnlyGuard,
    SkipsAnotherEventHook,
    SkipsUnrequestedGuardrail,
    RunsRequestedGuardrailWithoutDefaultOn,
    BlockedEntityFailsRequest,
    PerRequestLanguage,
    OutputParsePiiStoresTokens,
    ExampleRecognizers,
    NullRecognizerFile,
    KeepsTokensPreCallStored,
    GuardrailErrorStatus,
    MasksTexts,
    MasksTextsInOrder,
    AddsEmptyTexts,
    RestoresOnRestoreCallback,
    RestoresOnRolelessCallback,
    MasksRolelessCallbackWithoutTokens,
    MasksRolelessCallbackThatMasksOutput,
    MasksOnRequestInput,
    BlockedEntityFailsApply,
    HookWideningKeepsPostCallHook,
    PostCallUnmasksResponse,
    LoggingHookMasksRequest,
    AsyncLoggingHookMasksRequest,
    StreamingIteratorRelaysEvents,
    ZeroTimeoutUsesTheDefault,
    MalformedMockRedactedText,
    EmptyBasesFallBackToTheDefault,
    AnEmptyLanguageUsesTheDefault,
    ReturnsWhenMessagesAreNull,
    SkipsNonObjectContentParts,
    FailsWhenMetadataIsNotAnObject,
}

#[rstest]
#[case::defaults_are_applied(Case::DefaultsAreApplied)]
#[case::configured_values_are_applied(Case::ConfiguredValuesAreApplied)]
#[case::hook_widens_for_roleless_output_callback(Case::HookWidensForRolelessOutputCallback)]
#[case::hook_widens_for_roleless_output_mask_callback(
    Case::HookWidensForRolelessOutputMaskCallback
)]
#[case::hook_widening_appends_to_list(Case::HookWideningAppendsToList)]
#[case::hook_widening_keeps_list_with_post_call(Case::HookWideningKeepsListWithPostCall)]
#[case::hook_widening_skips_callback_with_role(Case::HookWideningSkipsCallbackWithRole)]
#[case::hook_widening_skips_roleless_callback_without_output(
    Case::HookWideningSkipsRolelessCallbackWithoutOutput
)]
#[case::hook_widening_keeps_tagged_mode(Case::HookWideningKeepsTaggedMode)]
#[case::logging_only_sets_event_hook(Case::LoggingOnlySetsEventHook)]
#[case::missing_recognizer_file(Case::MissingRecognizerFile)]
#[case::invalid_recognizer_json(Case::InvalidRecognizerJson)]
#[case::unreadable_recognizer_file(Case::UnreadableRecognizerFile)]
#[case::recognizer_file_is_not_a_list(Case::RecognizerFileIsNotAList)]
#[case::recognizer_file_is_read_first(Case::RecognizerFileIsReadFirst)]
#[case::missing_analyzer_base(Case::MissingAnalyzerBase)]
#[case::missing_anonymizer_base(Case::MissingAnonymizerBase)]
#[case::defaults_supply_api_bases(Case::DefaultsSupplyApiBases)]
#[case::explicit_base_wins(Case::ExplicitBaseWins)]
#[case::unrepresentable_timeout(Case::UnrepresentableTimeout)]
#[case::masks_string_content(Case::MasksStringContent)]
#[case::masks_list_content(Case::MasksListContent)]
#[case::skips_other_content(Case::SkipsOtherContent)]
#[case::returns_when_messages_are_absent(Case::ReturnsWhenMessagesAreAbsent)]
#[case::fails_when_messages_are_not_a_list(Case::FailsWhenMessagesAreNotAList)]
#[case::skips_logging_only_guard(Case::SkipsLoggingOnlyGuard)]
#[case::skips_another_event_hook(Case::SkipsAnotherEventHook)]
#[case::skips_unrequested_guardrail(Case::SkipsUnrequestedGuardrail)]
#[case::runs_requested_guardrail_without_default_on(Case::RunsRequestedGuardrailWithoutDefaultOn)]
#[case::blocked_entity_fails_request(Case::BlockedEntityFailsRequest)]
#[case::per_request_language(Case::PerRequestLanguage)]
#[case::output_parse_pii_stores_tokens(Case::OutputParsePiiStoresTokens)]
#[case::example_recognizers(Case::ExampleRecognizers)]
#[case::null_recognizer_file(Case::NullRecognizerFile)]
#[case::keeps_tokens_pre_call_stored(Case::KeepsTokensPreCallStored)]
#[case::guardrail_error_status(Case::GuardrailErrorStatus)]
#[case::masks_texts(Case::MasksTexts)]
#[case::masks_texts_in_order(Case::MasksTextsInOrder)]
#[case::adds_empty_texts(Case::AddsEmptyTexts)]
#[case::restores_on_restore_callback(Case::RestoresOnRestoreCallback)]
#[case::restores_on_roleless_callback(Case::RestoresOnRolelessCallback)]
#[case::masks_roleless_callback_without_tokens(Case::MasksRolelessCallbackWithoutTokens)]
#[case::masks_roleless_callback_that_masks_output(Case::MasksRolelessCallbackThatMasksOutput)]
#[case::masks_on_request_input(Case::MasksOnRequestInput)]
#[case::blocked_entity_fails_apply(Case::BlockedEntityFailsApply)]
#[case::hook_widening_keeps_a_post_call_hook(Case::HookWideningKeepsPostCallHook)]
#[case::post_call_unmasks_the_response(Case::PostCallUnmasksResponse)]
#[case::logging_hook_masks_the_request(Case::LoggingHookMasksRequest)]
#[case::async_logging_hook_masks_the_request(Case::AsyncLoggingHookMasksRequest)]
#[case::streaming_iterator_relays_events(Case::StreamingIteratorRelaysEvents)]
#[case::zero_timeout_uses_the_default(Case::ZeroTimeoutUsesTheDefault)]
#[case::malformed_mock_redacted_text(Case::MalformedMockRedactedText)]
#[case::empty_bases_fall_back_to_the_default(Case::EmptyBasesFallBackToTheDefault)]
#[case::an_empty_language_uses_the_default(Case::AnEmptyLanguageUsesTheDefault)]
#[case::returns_when_messages_are_null(Case::ReturnsWhenMessagesAreNull)]
#[case::skips_non_object_content_parts(Case::SkipsNonObjectContentParts)]
#[case::fails_when_metadata_is_not_an_object(Case::FailsWhenMetadataIsNotAnObject)]
#[tokio::test]
async fn presidio_runtime_cases(#[case] case: Case) {
    match case {
        Case::DefaultsAreApplied => construction::defaults_are_applied(),
        Case::ConfiguredValuesAreApplied => construction::configured_values_are_applied(),
        Case::HookWidensForRolelessOutputCallback => {
            construction::hook_widens_for_a_roleless_output_callback();
        }
        Case::HookWidensForRolelessOutputMaskCallback => {
            construction::hook_widens_for_a_roleless_output_mask_callback();
        }
        Case::HookWideningAppendsToList => construction::hook_widening_appends_to_a_list(),
        Case::HookWideningKeepsListWithPostCall => {
            construction::hook_widening_keeps_a_list_that_has_post_call();
        }
        Case::HookWideningSkipsCallbackWithRole => {
            construction::hook_widening_skips_a_callback_with_a_role();
        }
        Case::HookWideningSkipsRolelessCallbackWithoutOutput => {
            construction::hook_widening_skips_a_roleless_callback_without_output();
        }
        Case::HookWideningKeepsTaggedMode => construction::hook_widening_keeps_a_tagged_mode(),
        Case::LoggingOnlySetsEventHook => construction::logging_only_sets_the_event_hook(),
        Case::MissingRecognizerFile => construction::missing_recognizer_file_reports_the_path(),
        Case::InvalidRecognizerJson => {
            construction::invalid_recognizer_json_reports_the_decode_error()
        }
        Case::UnreadableRecognizerFile => {
            construction::unreadable_recognizer_file_reports_the_error()
        }
        Case::RecognizerFileIsNotAList => {
            construction::recognizer_file_that_is_not_a_list_reports_the_decode_error();
        }
        Case::RecognizerFileIsReadFirst => {
            construction::recognizer_file_is_read_before_the_api_bases()
        }
        Case::MissingAnalyzerBase => construction::missing_bases_report_the_analyzer_first(),
        Case::MissingAnonymizerBase => construction::a_missing_anonymizer_base_is_reported(),
        Case::DefaultsSupplyApiBases => construction::defaults_supply_the_api_bases(),
        Case::ExplicitBaseWins => construction::an_explicit_base_wins_over_the_default(),
        Case::UnrepresentableTimeout => construction::unrepresentable_timeout(),
        Case::MasksStringContent => masking::masks_string_content().await,
        Case::MasksListContent => masking::masks_list_content().await,
        Case::SkipsOtherContent => masking::skips_other_content().await,
        Case::ReturnsWhenMessagesAreAbsent => masking::returns_when_messages_are_absent().await,
        Case::FailsWhenMessagesAreNotAList => masking::fails_when_messages_are_not_a_list().await,
        Case::SkipsLoggingOnlyGuard => masking::skips_a_logging_only_guard().await,
        Case::SkipsAnotherEventHook => masking::skips_another_event_hook().await,
        Case::SkipsUnrequestedGuardrail => {
            masking::skips_a_guardrail_the_request_did_not_ask_for().await;
        }
        Case::RunsRequestedGuardrailWithoutDefaultOn => {
            masking::runs_a_requested_guardrail_without_default_on().await;
        }
        Case::BlockedEntityFailsRequest => masking::blocked_entity_fails_the_request().await,
        Case::PerRequestLanguage => masking::per_request_language_reaches_the_analyzer().await,
        Case::OutputParsePiiStoresTokens => masking::output_parse_pii_stores_tokens().await,
        Case::ExampleRecognizers => masking::example_recognizers_reach_the_analyze_request().await,
        Case::NullRecognizerFile => masking::a_null_recognizer_file_adds_no_recognizers().await,
        Case::KeepsTokensPreCallStored => masking::keeps_the_tokens_pre_call_stored().await,
        Case::GuardrailErrorStatus => masking::a_guardrail_error_status_is_reported().await,
        Case::MasksTexts => apply::masks_texts().await,
        Case::MasksTextsInOrder => apply::masks_texts_in_order().await,
        Case::AddsEmptyTexts => apply::adds_empty_texts().await,
        Case::RestoresOnRestoreCallback => apply::restores_on_a_restore_callback().await,
        Case::RestoresOnRolelessCallback => apply::restores_on_a_roleless_callback().await,
        Case::MasksRolelessCallbackWithoutTokens => {
            apply::masks_a_roleless_callback_without_tokens().await;
        }
        Case::MasksRolelessCallbackThatMasksOutput => {
            apply::masks_a_roleless_callback_that_masks_output().await;
        }
        Case::MasksOnRequestInput => apply::masks_on_a_request_input().await,
        Case::BlockedEntityFailsApply => apply::blocked_entity_fails_apply().await,
        Case::HookWideningKeepsPostCallHook => {
            construction::hook_widening_keeps_a_post_call_hook();
        }
        Case::PostCallUnmasksResponse => response::post_call_unmasks_the_response().await,
        Case::LoggingHookMasksRequest => response::logging_hook_masks_the_request(),
        Case::AsyncLoggingHookMasksRequest => {
            response::async_logging_hook_masks_the_request().await
        }
        Case::StreamingIteratorRelaysEvents => streaming::streaming_iterator_relays_events().await,
        Case::ZeroTimeoutUsesTheDefault => construction::zero_timeout_uses_the_default(),
        Case::MalformedMockRedactedText => {
            construction::malformed_mock_redacted_text_fails_construction();
        }
        Case::EmptyBasesFallBackToTheDefault => {
            construction::empty_bases_fall_back_to_the_default()
        }
        Case::AnEmptyLanguageUsesTheDefault => construction::an_empty_language_uses_the_default(),
        Case::ReturnsWhenMessagesAreNull => masking::returns_when_messages_are_null().await,
        Case::SkipsNonObjectContentParts => masking::skips_non_object_content_parts().await,
        Case::FailsWhenMetadataIsNotAnObject => {
            masking::fails_when_metadata_is_not_an_object().await;
        }
    }
}
