#![cfg(test)]

use rstest::rstest;

use super::is_tinyfish_async_route;

#[rstest]
#[case::the_async_route("https://agent.tinyfish.ai/v1/automation/run-async", true)]
#[case::the_async_route_with_a_query("https://agent.tinyfish.ai/v1/automation/run-async?x=1", true)]
#[case::a_prefixed_route("https://proxy.test/team/v1/automation/run-async", true)]
#[case::the_blocking_route("https://agent.tinyfish.ai/v1/automation/run", false)]
#[case::the_sse_route("https://agent.tinyfish.ai/v1/automation/run-sse", false)]
#[case::an_async_route_tail("https://agent.tinyfish.ai/v1/automation/run-async/extra", false)]
#[case::a_trailing_slash("https://agent.tinyfish.ai/v1/automation/run-async/", false)]
#[case::plain_text("not a url", false)]
#[case::an_empty_url("", false)]
#[case::an_unparsable_url("https://[::1/v1/automation/run-async", false)]
fn recognizes_the_async_route(#[case] url_route: &str, #[case] asynchronous: bool) {
    assert_eq!(is_tinyfish_async_route(url_route), asynchronous);
}
