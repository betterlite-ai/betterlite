#![cfg(test)]

mod capture;
mod cases;
mod logging;
mod sdk_cases;
mod support;

#[derive(Clone, Copy, Debug)]
enum TransportCase {
    BidirectionalRuntimeTarget,
    InitialSessionConfiguration,
    CredentialFailure,
    UpstreamSessionFailure,
    SessionCloseSequence,
    BridgeCompletionRace,
    PythonSdkBoundary,
}

#[rstest::rstest]
#[case::bidirectional_runtime_target(TransportCase::BidirectionalRuntimeTarget)]
#[case::initial_session_configuration(TransportCase::InitialSessionConfiguration)]
#[case::credential_failure(TransportCase::CredentialFailure)]
#[case::upstream_session_failure(TransportCase::UpstreamSessionFailure)]
#[case::session_close_sequence(TransportCase::SessionCloseSequence)]
#[case::bridge_completion_race(TransportCase::BridgeCompletionRace)]
#[case::python_sdk_boundary(TransportCase::PythonSdkBoundary)]
#[tokio::test]
async fn realtime_transport_cases(#[case] case: TransportCase) {
    match case {
        TransportCase::BidirectionalRuntimeTarget => cases::bidirectional_runtime_target().await,
        TransportCase::InitialSessionConfiguration => {
            cases::initial_session_configuration().await;
        }
        TransportCase::CredentialFailure => cases::credential_failure().await,
        TransportCase::UpstreamSessionFailure => cases::upstream_session_failure().await,
        TransportCase::SessionCloseSequence => cases::session_close_sequence().await,
        TransportCase::BridgeCompletionRace => cases::bridge_completion_race().await,
        TransportCase::PythonSdkBoundary => sdk_cases::python_sdk_boundary().await,
    }
}

#[tokio::test]
async fn bedrock_services_realtime_transport_credential_identity_isolation() {
    sdk_cases::credential_identity_isolation().await;
}

#[tokio::test]
async fn bedrock_services_realtime_transport_secret_safe_failure() {
    sdk_cases::secret_safe_failure().await;
}
