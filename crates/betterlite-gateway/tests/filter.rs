use betterlite_gateway::routes::GATEWAY_ROUTES;
use betterlite_server::server::filter::{ComponentRoutes, matches_path};
use rstest::rstest;

static EXACT: phf::Set<&'static str> = phf::phf_set! { "/exact" };
static MOUNTS: phf::Set<&'static str> = phf::phf_set! { "/mount" };

const TEST_ROUTES: ComponentRoutes = ComponentRoutes {
    prefixes: &[
        "/lit",
        "/tools/{name}/run/",
        "/item/{id}",
        "/open/{brace/",
        "/close/x{y}/",
    ],
    exact: &EXACT,
    mounts: &MOUNTS,
};

#[rstest]
#[case::mount_only("/mount", true)]
#[case::exact_only("/exact", true)]
#[case::plain_prefix_is_startswith("/litter", true)]
#[case::unlisted_path("/other", false)]
#[case::exact_is_not_a_prefix("/exactly", false)]
#[case::mount_is_not_a_prefix("/mounted", false)]
#[case::template_with_a_trailing_slash("/tools/web/run/now", true)]
#[case::template_needs_a_segment_after_the_slash("/tools/web/run", false)]
#[case::template_needs_every_segment("/tools/web", false)]
#[case::template_literal_must_match("/tools/web/walk/now", false)]
#[case::template_without_a_trailing_slash("/item/42", true)]
#[case::template_without_a_trailing_slash_needs_its_segment("/item", false)]
#[case::an_open_brace_alone_is_a_literal("/open/q/more", false)]
#[case::a_close_brace_alone_is_a_literal("/close/q/more", false)]
#[case::an_open_brace_alone_matches_itself("/open/{brace/more", true)]
fn matches_path_applies_mount_exact_and_prefix_rules(#[case] path: &str, #[case] allowed: bool) {
    assert_eq!(matches_path(path, TEST_ROUTES), allowed);
}

#[rstest]
#[case::exact_path("/routes", true)]
#[case::chat_prefix("/v1/chat/completions", true)]
#[case::provider_template("/acme/v1/files", true)]
#[case::provider_template_needs_a_second_segment("/not-a-route", false)]
fn gateway_routes_follow_the_allowlists(#[case] path: &str, #[case] allowed: bool) {
    assert_eq!(matches_path(path, GATEWAY_ROUTES), allowed);
}
