#![cfg(test)]

use rstest::rstest;

mod cases;
mod fixture;
mod message_cap;

#[derive(Debug, Clone, Copy)]
enum Case {
    BetaRealtimeDeploymentQuery,
    GaRealtimeUrl,
    ApiVersionQuery,
    ApiKeyVersusToken,
    ClientBetaProtocol,
    CallsAndTranscriptionHeaders,
    HandshakeError,
    HttpVersion,
}

#[rstest]
#[case::beta_realtime_deployment_query(Case::BetaRealtimeDeploymentQuery)]
#[case::ga_realtime_url(Case::GaRealtimeUrl)]
#[case::api_version_query(Case::ApiVersionQuery)]
#[case::api_key_versus_token(Case::ApiKeyVersusToken)]
#[case::client_beta_protocol(Case::ClientBetaProtocol)]
#[case::calls_and_transcription_headers(Case::CallsAndTranscriptionHeaders)]
#[case::handshake_error(Case::HandshakeError)]
#[case::http_version_2024_12_17(Case::HttpVersion)]
fn realtime_cases(#[case] case: Case) {
    match case {
        Case::BetaRealtimeDeploymentQuery => cases::beta_realtime_deployment_query(),
        Case::GaRealtimeUrl => cases::ga_realtime_url(),
        Case::ApiVersionQuery => cases::api_version_query(),
        Case::ApiKeyVersusToken => cases::api_key_versus_token(),
        Case::ClientBetaProtocol => cases::client_beta_protocol(),
        Case::CallsAndTranscriptionHeaders => cases::calls_and_transcription_headers(),
        Case::HandshakeError => cases::handshake_error(),
        Case::HttpVersion => cases::http_version_2024_12_17(),
    }
}
