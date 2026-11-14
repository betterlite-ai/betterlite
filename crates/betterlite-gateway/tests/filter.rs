#![cfg(test)]
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use axum::middleware::from_fn;
use axum::routing::get;
use axum::{Extension, Router};
use betterlite_gateway::routes::GATEWAY_ROUTES;
use betterlite_server::server::filter::{ComponentRoutes, filter_route, keeps_route};
use rstest::rstest;
use tower::ServiceExt as _;

static EXACT: phf::Set<&'static str> = phf::phf_set! { "/exact" };
static MOUNTS: phf::Set<&'static str> = phf::phf_set! { "/mount" };

const TEST_ROUTES: ComponentRoutes = ComponentRoutes {
    prefixes: &["/lit", "/{provider}/"],
    exact: &EXACT,
    mounts: &MOUNTS,
};

#[rstest]
#[case::exact_only("/exact", true)]
#[case::exact_is_not_a_prefix("/exactly", false)]
#[case::plain_prefix_is_startswith("/litter", true)]
#[case::unlisted_template("/other", false)]
#[case::mount_only("/mount", true)]
#[case::mount_is_not_a_prefix("/mounted", false)]
#[case::mount_keeps_its_slash_route("/mount/", true)]
#[case::mount_keeps_its_wildcard_route("/mount/{*path}", true)]
#[case::template_prefix_is_matched_as_text("/{provider}/v1/files", true)]
#[case::template_prefix_needs_the_same_name("/{tenant}/v1/files", false)]
#[case::literal_segment_is_not_a_template("/acme/v1/files", false)]
#[case::template_prefix_must_lead("/key/{provider}/files", false)]
fn keeps_route_applies_exact_prefix_and_mount_rules(#[case] template: &str, #[case] kept: bool) {
    assert_eq!(keeps_route(template, TEST_ROUTES), kept);
}

#[rstest]
#[case::exact_path("/routes", true)]
#[case::chat_prefix("/v1/chat/completions", true)]
#[case::provider_prefix("/openai/v1/files", true)]
#[case::provider_template("/{provider}/v1/files", true)]
#[case::mount_path("/metrics", true)]
#[case::management_route("/key/generate", false)]
#[case::management_user_route("/user/new", false)]
#[case::management_access_group("/v1/access_group", false)]
#[case::mcp_server_template("/{mcp_server_name}/authorize", false)]
fn gateway_routes_follow_the_allowlists(#[case] template: &str, #[case] kept: bool) {
    assert_eq!(keeps_route(template, GATEWAY_ROUTES), kept);
}

fn application() -> Router {
    Router::new()
        .route("/exact", get(|| async { "kept" }))
        .route("/blocked", get(|| async { "kept" }))
        .route("/key/generate", get(|| async { "kept" }))
        .route("/{provider}/v1/files", get(|| async { "kept" }))
        .route("/zz/{provider}", get(|| async { "kept" }))
        .layer(from_fn(filter_route))
}

async fn status_and_body(application: Router, path: &str) -> (StatusCode, String) {
    let request = Request::builder().uri(path).body(Body::empty()).unwrap();
    let response = application.oneshot(request).await.unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

#[rstest]
#[case::exact_route("/exact", StatusCode::OK, "kept")]
#[case::route_outside_the_allowlist("/blocked", StatusCode::NOT_FOUND, r#"{"detail":"Not Found"}"#)]
#[case::template_under_a_kept_prefix("/acme/v1/files", StatusCode::OK, "kept")]
#[case::template_outside_every_prefix(
    "/zz/acme",
    StatusCode::NOT_FOUND,
    r#"{"detail":"Not Found"}"#
)]
#[case::two_segment_path_of_another_route(
    "/key/generate",
    StatusCode::NOT_FOUND,
    r#"{"detail":"Not Found"}"#
)]
#[tokio::test]
async fn filter_route_decides_by_the_matched_template(
    #[case] path: &str,
    #[case] status: StatusCode,
    #[case] body: &str,
) {
    let routed = application().layer(Extension(TEST_ROUTES));
    assert_eq!(
        status_and_body(routed, path).await,
        (status, body.to_owned())
    );
}

#[tokio::test]
async fn filter_route_keeps_every_route_without_an_allowlist() {
    let (status, body) = status_and_body(application(), "/blocked").await;
    assert_eq!((status, body.as_str()), (StatusCode::OK, "kept"));
}

#[tokio::test]
async fn filter_route_leaves_a_request_that_matches_no_route_to_the_application() {
    let routed = application().layer(Extension(TEST_ROUTES));
    let (status, _) = status_and_body(routed, "/a/b/c").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
