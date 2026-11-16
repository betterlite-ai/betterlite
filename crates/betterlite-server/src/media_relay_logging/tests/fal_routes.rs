#![cfg(test)]

use betterlite_core::api::media_relay_logging::routes::is_fal_ai_route;
use rstest::rstest;

#[rstest]
#[case::provider_match("https://gateway.example/v1/images", Some("fal_ai"), true)]
#[case::any_route_with_the_provider("not a url at all", Some("fal_ai"), true)]
#[case::fal_route_without_provider("https://queue.fal.run/fal-ai/flux/dev", None, false)]
#[case::other_provider("https://queue.fal.run/fal-ai/flux/dev", Some("openai"), false)]
#[case::provider_suffix("https://queue.fal.run/fal-ai/flux/dev", Some("fal_ai_extra"), false)]
fn fal_ai_route_reads_the_provider(
    #[case] route: &str,
    #[case] provider: Option<&str>,
    #[case] expected: bool,
) {
    assert_eq!(is_fal_ai_route(route, provider), expected);
}
