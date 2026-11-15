#![cfg(test)]

mod construction;
mod logging;
mod scans;
mod support;

use rstest::rstest;

#[derive(Debug, Clone, Copy)]
enum Case {
    ConstructorPrecedence,
    EnvironmentFallbacks,
    UnsetDefaults,
    MissingKey,
    DefaultPolicyList,
    ConfiguredPolicyList,
    SafeRequestScan,
    UnsafeRequestScan,
    SafeResponseScanWithoutGrounding,
    UnsafeResponseScan,
    GroundingBlock,
    BlockOnErrorTrue,
    BlockOnErrorFalse,
    LoggingSkipsExistingInformation,
    LoggingScansEmptyInformation,
    LoggingAppendsResponseEntry,
    LoggingAppendsRequestEntry,
    LoggingEmptyHistory,
    LoggingFailedScan,
    LoggingSwallowedFailure,
    EmptyHistory,
    DefaultGuardrailName,
    GroundingFailureFailsOpen,
    GroundingFailureBlocks,
    LoggingScansNullInformation,
    LoggingSkipsStringInformation,
    LoggingReplacesNonListEntry,
    LoggingAppendsToExistingList,
    LoggingSwallowedNonObject,
    GroundingPasses,
    UnrepresentableTimeout,
    RequestIgnoresGroundingDocuments,
    LoggingRedactsSecretFields,
}

#[rstest]
#[case::constructor_precedence(Case::ConstructorPrecedence)]
#[case::environment_fallbacks(Case::EnvironmentFallbacks)]
#[case::unset_defaults(Case::UnsetDefaults)]
#[case::missing_key(Case::MissingKey)]
#[case::default_policy_list(Case::DefaultPolicyList)]
#[case::configured_policy_list(Case::ConfiguredPolicyList)]
#[case::safe_request_scan(Case::SafeRequestScan)]
#[case::unsafe_request_scan(Case::UnsafeRequestScan)]
#[case::safe_response_scan_without_grounding(Case::SafeResponseScanWithoutGrounding)]
#[case::unsafe_response_scan(Case::UnsafeResponseScan)]
#[case::grounding_block(Case::GroundingBlock)]
#[case::block_on_error_true(Case::BlockOnErrorTrue)]
#[case::block_on_error_false(Case::BlockOnErrorFalse)]
#[case::logging_skips_existing_information(Case::LoggingSkipsExistingInformation)]
#[case::logging_scans_empty_information(Case::LoggingScansEmptyInformation)]
#[case::logging_appends_response_entry(Case::LoggingAppendsResponseEntry)]
#[case::logging_appends_request_entry(Case::LoggingAppendsRequestEntry)]
#[case::logging_empty_history(Case::LoggingEmptyHistory)]
#[case::logging_failed_scan(Case::LoggingFailedScan)]
#[case::logging_swallowed_failure(Case::LoggingSwallowedFailure)]
#[case::empty_history(Case::EmptyHistory)]
#[case::default_guardrail_name(Case::DefaultGuardrailName)]
#[case::grounding_failure_fails_open(Case::GroundingFailureFailsOpen)]
#[case::grounding_failure_blocks(Case::GroundingFailureBlocks)]
#[case::logging_scans_null_information(Case::LoggingScansNullInformation)]
#[case::logging_skips_string_information(Case::LoggingSkipsStringInformation)]
#[case::logging_replaces_non_list_entry(Case::LoggingReplacesNonListEntry)]
#[case::logging_appends_to_existing_list(Case::LoggingAppendsToExistingList)]
#[case::logging_swallowed_non_object(Case::LoggingSwallowedNonObject)]
#[case::grounding_passes(Case::GroundingPasses)]
#[case::unrepresentable_timeout(Case::UnrepresentableTimeout)]
#[case::request_ignores_grounding_documents(Case::RequestIgnoresGroundingDocuments)]
#[case::logging_redacts_secret_fields(Case::LoggingRedactsSecretFields)]
#[tokio::test]
async fn xecguard_runtime_cases(#[case] case: Case) {
    match case {
        Case::ConstructorPrecedence => construction::constructor_precedence(),
        Case::EnvironmentFallbacks => construction::environment_fallbacks(),
        Case::UnsetDefaults => construction::unset_defaults(),
        Case::MissingKey => construction::missing_key(),
        Case::DefaultPolicyList => construction::default_policy_list().await,
        Case::ConfiguredPolicyList => construction::configured_policy_list().await,
        Case::UnrepresentableTimeout => construction::unrepresentable_timeout(),
        Case::SafeRequestScan => scans::safe_request_scan().await,
        Case::UnsafeRequestScan => scans::unsafe_request_scan().await,
        Case::SafeResponseScanWithoutGrounding => scans::safe_response_scan().await,
        Case::UnsafeResponseScan => scans::unsafe_response_scan().await,
        Case::GroundingBlock => scans::grounding_block().await,
        Case::BlockOnErrorTrue => scans::block_on_error_true().await,
        Case::BlockOnErrorFalse => scans::block_on_error_false().await,
        Case::EmptyHistory => scans::empty_history().await,
        Case::DefaultGuardrailName => scans::default_guardrail_name().await,
        Case::GroundingFailureFailsOpen => scans::grounding_failure_fails_open().await,
        Case::GroundingFailureBlocks => scans::grounding_failure_blocks().await,
        Case::GroundingPasses => scans::grounding_passes().await,
        Case::RequestIgnoresGroundingDocuments => {
            scans::request_ignores_grounding_documents().await
        }
        Case::LoggingSkipsExistingInformation => logging::logging_skips_existing().await,
        Case::LoggingScansEmptyInformation => logging::logging_scans_empty_information().await,
        Case::LoggingAppendsResponseEntry => logging::logging_appends_response_entry().await,
        Case::LoggingAppendsRequestEntry => logging::logging_appends_request_entry().await,
        Case::LoggingEmptyHistory => logging::logging_empty_history().await,
        Case::LoggingFailedScan => logging::logging_failed_scan().await,
        Case::LoggingSwallowedFailure => logging::logging_swallowed_failure().await,
        Case::LoggingScansNullInformation => logging::logging_scans_null_information().await,
        Case::LoggingSkipsStringInformation => logging::logging_skips_string_information().await,
        Case::LoggingReplacesNonListEntry => logging::logging_replaces_non_list_entry().await,
        Case::LoggingAppendsToExistingList => logging::logging_appends_to_existing_list().await,
        Case::LoggingSwallowedNonObject => logging::logging_swallowed_non_object().await,
        Case::LoggingRedactsSecretFields => logging::logging_redacts_secret_fields().await,
    }
}
