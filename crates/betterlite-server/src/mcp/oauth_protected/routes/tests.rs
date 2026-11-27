use axum::Router;
use axum::http::StatusCode;
use axum::routing::get;

use crate::mcp::oauth_protected::tests::support::{
    AGGREGATE_AUTH_SERVER_DOCUMENT, AGGREGATE_AUTH_SERVER_WITH_EXCHANGE_DOCUMENT,
    AGGREGATE_RESOURCE_DOCUMENT, GATEWAY_LEGACY_DOCUMENT, GATEWAY_STANDARD_DOCUMENT, Harness,
    PASS_THROUGH_HOST, PASS_THROUGH_PATH, ROOT_DOCUMENT, exchange_available, gateway_server,
    passthrough_server, send_get,
};

#[tokio::test]
async fn the_root_route_answers_the_root_document() {
    let router = Harness::new().router();
    let reply = send_get(&router, "/.well-known/oauth-protected-resource").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body, ROOT_DOCUMENT);
}

#[tokio::test]
async fn the_aggregate_resource_route_answers_the_aggregate_document() {
    let router = Harness::new().router();
    let reply = send_get(&router, "/.well-known/oauth-protected-resource/mcp").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body, AGGREGATE_RESOURCE_DOCUMENT);
}

#[tokio::test]
async fn the_suffixed_routes_move_with_the_suffix() {
    let harness = Harness::new()
        .suffix("-tenant")
        .servers(vec![gateway_server()]);
    let router = harness.router();
    let root = send_get(&router, "/.well-known/oauth-protected-resource").await;
    assert_eq!(root.status, StatusCode::OK);
    assert_eq!(root.body, ROOT_DOCUMENT);
    let aggregate = send_get(&router, "/.well-known/oauth-protected-resource-tenant/mcp").await;
    assert_eq!(aggregate.status, StatusCode::OK);
    assert_eq!(aggregate.body, AGGREGATE_RESOURCE_DOCUMENT);
    let authorization_server = send_get(
        &router,
        "/.well-known/oauth-authorization-server-tenant/mcp",
    )
    .await;
    assert_eq!(authorization_server.status, StatusCode::OK);
    assert_eq!(authorization_server.body, AGGREGATE_AUTH_SERVER_DOCUMENT);
    let standard = send_get(
        &router,
        "/.well-known/oauth-protected-resource-tenant/mcp/gateway",
    )
    .await;
    assert_eq!(standard.status, StatusCode::OK);
    assert_eq!(standard.body, GATEWAY_STANDARD_DOCUMENT);
    let legacy = send_get(
        &router,
        "/.well-known/oauth-protected-resource-tenant/gateway/mcp",
    )
    .await;
    assert_eq!(legacy.status, StatusCode::OK);
    assert_eq!(legacy.body, GATEWAY_LEGACY_DOCUMENT);
    let unsuffixed = send_get(&router, "/.well-known/oauth-protected-resource/mcp").await;
    assert_eq!(unsuffixed.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_aggregate_authorization_server_lists_token_exchange_when_available() {
    let router = Harness::new().token_exchange(exchange_available()).router();
    let reply = send_get(&router, "/.well-known/oauth-authorization-server/mcp").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body, AGGREGATE_AUTH_SERVER_WITH_EXCHANGE_DOCUMENT);
}

#[tokio::test]
async fn the_standard_and_legacy_routes_name_their_own_resource() {
    let router = Harness::new().servers(vec![gateway_server()]).router();
    let standard = send_get(&router, "/.well-known/oauth-protected-resource/mcp/gateway").await;
    assert_eq!(standard.status, StatusCode::OK);
    assert_eq!(standard.body, GATEWAY_STANDARD_DOCUMENT);
    let legacy = send_get(&router, "/.well-known/oauth-protected-resource/gateway/mcp").await;
    assert_eq!(legacy.status, StatusCode::OK);
    assert_eq!(legacy.body, GATEWAY_LEGACY_DOCUMENT);
}

#[tokio::test]
async fn a_server_named_mcp_keeps_its_named_route() {
    let mut server = gateway_server();
    server.fields.name = String::from("mcp");
    let router = Harness::new().servers(vec![server]).router();
    let reply = send_get(&router, "/.well-known/oauth-protected-resource/mcp/mcp").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.body,
        r#"{"authorization_servers":["http://litellm.internal/mcp"],"resource":"http://litellm.internal/mcp/mcp","scopes_supported":[]}"#
    );
}

#[tokio::test]
async fn an_unknown_server_is_a_404() {
    let router = Harness::new().servers(vec![gateway_server()]).router();
    let reply = send_get(&router, "/.well-known/oauth-protected-resource/mcp/ghost").await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(
        reply.body,
        r#"{"detail":"MCP server 'ghost' is not an OAuth-protected resource"}"#
    );
}

#[tokio::test]
async fn the_server_also_resolves_by_id() {
    let router = Harness::new().servers(vec![gateway_server()]).router();
    let reply = send_get(
        &router,
        "/.well-known/oauth-protected-resource/mcp/srv-gateway",
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.body,
        r#"{"authorization_servers":["http://litellm.internal/mcp"],"resource":"http://litellm.internal/mcp/srv-gateway","scopes_supported":[]}"#
    );
}

#[tokio::test]
async fn an_internal_server_is_hidden_from_a_public_client() {
    let mut server = gateway_server();
    server.fields.available_on_public_internet = false;
    let public = Harness::new()
        .servers(vec![server.clone()])
        .client_ip("8.8.8.8")
        .router();
    let hidden = send_get(&public, "/.well-known/oauth-protected-resource/mcp/gateway").await;
    assert_eq!(hidden.status, StatusCode::NOT_FOUND);
    let internal = Harness::new()
        .servers(vec![server])
        .client_ip("127.0.0.1")
        .router();
    let visible = send_get(
        &internal,
        "/.well-known/oauth-protected-resource/mcp/gateway",
    )
    .await;
    assert_eq!(visible.status, StatusCode::OK);
}

#[tokio::test]
async fn a_pass_through_failure_is_a_502_with_the_detail_body() {
    let harness = Harness::new()
        .servers(vec![passthrough_server()])
        .status(PASS_THROUGH_HOST, 404)
        .status(PASS_THROUGH_PATH, 404);
    let router = harness.router();
    let reply = send_get(
        &router,
        "/.well-known/oauth-protected-resource/mcp/upstream",
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_GATEWAY);
    assert_eq!(
        reply.body,
        r#"{"detail":"Upstream oauth-protected-resource metadata unavailable for MCP server 'upstream'"}"#
    );
}

#[tokio::test]
async fn the_aggregate_authorization_server_route_is_one_segment() {
    let router = Harness::new().servers(vec![gateway_server()]).router();
    let deeper = send_get(
        &router,
        "/.well-known/oauth-authorization-server/mcp/gateway",
    )
    .await;
    assert_eq!(deeper.status, StatusCode::NOT_FOUND);
    let legacy = send_get(
        &router,
        "/.well-known/oauth-authorization-server/gateway/mcp",
    )
    .await;
    assert_eq!(legacy.status, StatusCode::NOT_FOUND);
    let bare = send_get(&router, "/.well-known/oauth-authorization-server").await;
    assert_eq!(bare.status, StatusCode::NOT_FOUND);
}

fn other_authorization_server_router() -> Router {
    Router::new()
        .route(
            "/.well-known/oauth-authorization-server/{mcp_server_name}",
            get(other_document),
        )
        .route(
            "/.well-known/oauth-authorization-server/mcp/{mcp_server_name}",
            get(other_document),
        )
        .route(
            "/.well-known/oauth-authorization-server/{mcp_server_name}/mcp",
            get(other_document),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            get(other_document),
        )
}

async fn other_document() -> &'static str {
    "other"
}

#[tokio::test]
async fn the_router_merges_with_the_authorization_server_routes() {
    let router = Harness::new()
        .servers(vec![gateway_server()])
        .router()
        .merge(other_authorization_server_router());
    let aggregate = send_get(&router, "/.well-known/oauth-authorization-server/mcp").await;
    assert_eq!(aggregate.status, StatusCode::OK);
    assert_eq!(aggregate.body, AGGREGATE_AUTH_SERVER_DOCUMENT);
    let named = send_get(&router, "/.well-known/oauth-authorization-server/gateway").await;
    assert_eq!(named.body, "other");
    let standard = send_get(
        &router,
        "/.well-known/oauth-authorization-server/mcp/gateway",
    )
    .await;
    assert_eq!(standard.body, "other");
    let legacy = send_get(
        &router,
        "/.well-known/oauth-authorization-server/gateway/mcp",
    )
    .await;
    assert_eq!(legacy.body, "other");
    let bare = send_get(&router, "/.well-known/oauth-authorization-server").await;
    assert_eq!(bare.body, "other");
}
