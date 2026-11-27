mod tests;

use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Path, Request, State};
use axum::response::{IntoResponse as _, Response};
use betterlite_mcp::registry::lookup::ServerLookups as _;
use betterlite_mcp::registry::servers::SharedRegistry;
use betterlite_mcp::server_record::MCPServer;
use serde::Serialize;

use super::documents::{aggregate_auth_server, aggregate_protected_resource, root_resource};
use super::{GatewayFacts, ProtectedRoot, RequestFacts, UpstreamGet};

pub(crate) const PROTECTED_RESOURCE_PATH: &str = "/.well-known/oauth-protected-resource";
pub(crate) const AUTHORIZATION_SERVER_PATH: &str = "/.well-known/oauth-authorization-server";

#[derive(Clone, Copy)]
pub(crate) enum NamedShape {
    Standard,
    Legacy,
}

#[derive(Clone, Copy)]
pub(crate) enum WellKnownShape {
    Root,
    AggregateResource,
    AggregateAuthorizationServer,
}

#[derive(Serialize)]
struct DetailBody {
    detail: String,
}

pub(crate) async fn well_known_document<U, R, G>(
    State(root): State<Arc<ProtectedRoot<U, R, G>>>,
    Extension(shape): Extension<WellKnownShape>,
    request: Request,
) -> Response
where
    U: UpstreamGet + 'static,
    R: RequestFacts + 'static,
    G: GatewayFacts + 'static,
{
    let (parts, _body) = request.into_parts();
    let base_url = root.request.request_base_url(&parts);
    match shape {
        WellKnownShape::Root => Json(root_resource(&base_url)).into_response(),
        WellKnownShape::AggregateResource => {
            Json(aggregate_protected_resource(&base_url)).into_response()
        }
        WellKnownShape::AggregateAuthorizationServer => Json(aggregate_auth_server(
            &base_url,
            root.gateway.token_exchange().refusal().is_none(),
        ))
        .into_response(),
    }
}

pub(crate) async fn named_document<U, R, G>(
    State(root): State<Arc<ProtectedRoot<U, R, G>>>,
    Extension(shape): Extension<NamedShape>,
    Path(name): Path<String>,
    request: Request,
) -> Response
where
    U: UpstreamGet + 'static,
    R: RequestFacts + 'static,
    G: GatewayFacts + 'static,
{
    let (parts, _body) = request.into_parts();
    let base_url = root.request.request_base_url(&parts);
    let client_ip = root.request.client_ip(&parts);
    let server = resolve_server(&root.registry, &name, client_ip.as_deref()).await;
    let standard = matches!(shape, NamedShape::Standard);
    match root
        .named_resource(&base_url, &name, standard, server.as_ref())
        .await
    {
        Ok(document) => Json(document).into_response(),
        Err(failure) => (
            failure.status,
            Json(DetailBody {
                detail: failure.detail,
            }),
        )
            .into_response(),
    }
}

async fn resolve_server(
    registry: &SharedRegistry,
    name: &str,
    client_ip: Option<&str>,
) -> Option<MCPServer> {
    let servers = registry.read().await;
    servers
        .server_by_name(name, client_ip)
        .or_else(|| servers.server_by_id(name, client_ip))
        .cloned()
}
