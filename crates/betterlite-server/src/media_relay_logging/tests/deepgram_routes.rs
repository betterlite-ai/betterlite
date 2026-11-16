#![cfg(test)]

use betterlite_core::api::media_relay_logging::routes::is_deepgram_listen_route;
use rstest::rstest;

#[rstest]
#[case::proxy_route("https://gateway.example/deepgram/v1/listen", true)]
#[case::query_is_not_the_path("https://gateway.example/deepgram/v1/listen?model=nova-3", true)]
#[case::listen_path_only("https://api.deepgram.com/v1/listen", false)]
#[case::path_continues_after_listen("https://gateway.example/deepgram/v1/listen/extra", false)]
#[case::other_endpoint("https://gateway.example/deepgram/v1/speak", false)]
#[case::capitalized_suffix("https://gateway.example/deepgram/v1/Listen", false)]
#[case::segment_spelled_out("https://gateway.example/deepgram-listen", false)]
#[case::segment_only_in_the_query(
    "https://api.deepgram.com/v1/listen?next=/deepgram/listen",
    false
)]
fn deepgram_listen_route_reads_the_proxy_path(#[case] route: &str, #[case] expected: bool) {
    assert_eq!(is_deepgram_listen_route(route), expected);
}
