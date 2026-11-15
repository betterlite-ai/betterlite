#![cfg(test)]

use rstest::rstest;

mod cases;
mod fixture;
mod message_cap;

#[derive(Debug, Clone, Copy)]
enum Case {
    WsWssScheme,
    ReplacePathWithV1Realtime,
    QueryModelOrExplicit,
    BetaHeaderOnlyWhenClientSends,
    HttpSessionPaths,
    MissingApiKey,
    HandshakeRefusal,
    RedactedCloseReason,
    RealtimeSslAndSize,
    RealtimeHttpAuth,
}

#[rstest]
#[case::ws_wss_scheme(Case::WsWssScheme)]
#[case::replace_path_with_v1_realtime(Case::ReplacePathWithV1Realtime)]
#[case::query_model_or_explicit(Case::QueryModelOrExplicit)]
#[case::beta_header_only_when_client_sends(Case::BetaHeaderOnlyWhenClientSends)]
#[case::http_session_paths(Case::HttpSessionPaths)]
#[case::missing_api_key(Case::MissingApiKey)]
#[case::handshake_refusal(Case::HandshakeRefusal)]
#[case::redacted_close_reason(Case::RedactedCloseReason)]
#[case::realtime_ssl_and_size(Case::RealtimeSslAndSize)]
#[case::realtime_http_auth(Case::RealtimeHttpAuth)]
fn realtime_cases(#[case] case: Case) {
    match case {
        Case::WsWssScheme => cases::ws_wss_scheme(),
        Case::ReplacePathWithV1Realtime => cases::replace_path_with_v1_realtime(),
        Case::QueryModelOrExplicit => cases::query_model_or_explicit(),
        Case::BetaHeaderOnlyWhenClientSends => cases::beta_header_only_when_client_sends(),
        Case::HttpSessionPaths => cases::http_session_paths(),
        Case::MissingApiKey => cases::missing_api_key(),
        Case::HandshakeRefusal => cases::handshake_refusal(),
        Case::RedactedCloseReason => cases::redacted_close_reason(),
        Case::RealtimeSslAndSize => cases::realtime_ssl_and_size(),
        Case::RealtimeHttpAuth => cases::realtime_http_auth(),
    }
}
