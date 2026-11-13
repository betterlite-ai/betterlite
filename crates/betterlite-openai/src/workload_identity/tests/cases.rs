use rstest::rstest;

use super::{decoding, failures, flight, flows, registry, resolution};

#[rstest]
#[case::static_key_disables_wif(0)]
#[case::three_fields_required(1)]
#[case::https_openai_host_only(2)]
#[case::kubernetes_token_exchange(3)]
#[case::token_refresh_and_error(4)]
#[case::exchange_json_fields(5)]
#[case::refresh_half_life_boundary(6)]
#[case::empty_subject_token(7)]
#[case::oauth_status_errors(8)]
#[case::concurrent_refresh_single_flight(9)]
#[case::identity_cache_boundary(10)]
#[case::sdk_auth_unavailable_500(11)]
#[tokio::test]
async fn workload_identity_cases(#[case] case: u8) {
    match case {
        0 => resolution::static_key_disables_wif(),
        1 => resolution::three_fields_required(),
        2 => resolution::https_openai_host_only(),
        3 => flows::kubernetes_token_exchange().await,
        4 => flows::token_refresh_and_error().await,
        5 => decoding::exchange_json_fields(),
        6 => flows::refresh_half_life_boundary().await,
        7 => failures::empty_subject_token().await,
        8 => failures::oauth_status_errors().await,
        9 => flight::concurrent_refresh_single_flight().await,
        10 => registry::identity_cache_boundary(),
        _ => registry::sdk_auth_unavailable_500(),
    }
}
