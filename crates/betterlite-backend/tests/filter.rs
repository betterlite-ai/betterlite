#![cfg(test)]
use betterlite_backend::BACKEND_ROUTES;
use betterlite_server::server::filter::keeps_route;
use rstest::rstest;

#[rstest]
#[case::exact_path("/routes", true)]
#[case::management_prefix("/key/generate", true)]
#[case::management_user_prefix("/user/new", true)]
#[case::access_group("/v1/access_group", true)]
#[case::mcp_server_template("/{mcp_server_name}/authorize", true)]
#[case::mount_path("/swagger", true)]
#[case::mount_wildcard_route("/swagger/{*path}", true)]
#[case::mount_is_not_a_prefix("/swaggerx", false)]
#[case::chat_completions("/v1/chat/completions", false)]
#[case::provider_prefix("/openai/v1/files", false)]
#[case::provider_template("/{provider}/v1/files", false)]
fn backend_routes_follow_the_allowlists(#[case] template: &str, #[case] kept: bool) {
    assert_eq!(keeps_route(template, BACKEND_ROUTES), kept);
}
