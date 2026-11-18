#![cfg(test)]

mod cancel_cases;
mod client_cases;
mod close_cases;
mod provider_cases;
mod support;

#[derive(Clone, Copy, Debug)]
enum BridgeCase {
    ClientEventForwarding,
    ProviderEventForwarding,
    ProviderErrorClose,
    InputHalfClose,
    OutputHalfClose,
    CancelSession,
}

#[rstest::rstest]
#[case::client_event_forwarding(BridgeCase::ClientEventForwarding)]
#[case::provider_event_forwarding(BridgeCase::ProviderEventForwarding)]
#[case::provider_error_close(BridgeCase::ProviderErrorClose)]
#[case::input_half_close(BridgeCase::InputHalfClose)]
#[case::output_half_close(BridgeCase::OutputHalfClose)]
#[case::cancel_session(BridgeCase::CancelSession)]
#[tokio::test]
async fn realtime_bridge_cases(#[case] case: BridgeCase) {
    match case {
        BridgeCase::ClientEventForwarding => client_cases::client_event_forwarding().await,
        BridgeCase::ProviderEventForwarding => provider_cases::provider_event_forwarding().await,
        BridgeCase::ProviderErrorClose => provider_cases::provider_error_close().await,
        BridgeCase::InputHalfClose => close_cases::input_half_close().await,
        BridgeCase::OutputHalfClose => close_cases::output_half_close().await,
        BridgeCase::CancelSession => cancel_cases::cancel_session().await,
    }
}
