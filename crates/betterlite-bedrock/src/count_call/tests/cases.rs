#![cfg(test)]

use rstest::rstest;

#[derive(Debug, Clone, Copy)]
enum Scenario {
    CountTokenHttpRequest,
    NativeCountSelection,
    CountApiFailureWarning,
    CountCredentials,
    CountTimeout,
    CounterFactory,
    CountResultOrNone,
    CountDebugEvents,
}

#[rstest]
#[case::count_token_http_request(Scenario::CountTokenHttpRequest)]
#[case::native_count_selection(Scenario::NativeCountSelection)]
#[case::count_api_failure_warning(Scenario::CountApiFailureWarning)]
#[case::count_credentials(Scenario::CountCredentials)]
#[case::count_timeout(Scenario::CountTimeout)]
#[case::counter_factory(Scenario::CounterFactory)]
#[case::count_result_or_none(Scenario::CountResultOrNone)]
#[case::count_debug_events(Scenario::CountDebugEvents)]
#[tokio::test]
async fn count_call_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::CountTokenHttpRequest => super::http_call_cases::count_token_http_request().await,
        Scenario::NativeCountSelection => super::selection_cases::native_count_selection().await,
        Scenario::CountApiFailureWarning => {
            super::failure_cases::count_api_failure_warning().await;
        }
        Scenario::CountCredentials => super::credential_cases::count_credentials().await,
        Scenario::CountTimeout => super::timeout_cases::count_timeout().await,
        Scenario::CounterFactory => super::factory_cases::counter_factory().await,
        Scenario::CountResultOrNone => super::result_cases::count_result_or_none().await,
        Scenario::CountDebugEvents => super::debug_cases::count_debug_events().await,
    }
}
