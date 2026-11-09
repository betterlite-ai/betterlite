use std::collections::BTreeSet;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{OriginalUri, Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse as _, Redirect, Response};
use axum::routing::{any, get};
use betterlite_openapi::docs_pages::{
    SwaggerPageInputs, oauth2_redirect_page, redoc_page, swagger_asset, swagger_ui_page,
};
use betterlite_openapi::docs_settings::DocsUrls;

mod tests;

pub struct DocsRouteConfig {
    pub urls: DocsUrls,
    pub title: String,
    pub server_root_path: String,
}

pub fn docs_router(config: DocsRouteConfig) -> Router {
    // the caller registers the OpenAPI route, and FastAPI adds it before these pages
    let mut paths: BTreeSet<String> = config.urls.openapi_url.iter().cloned().collect();
    let mut router: Router<Arc<DocsRouteConfig>> = Router::new();
    router = add_page_routes(router, &config, &mut paths);
    router = add_swagger_mount(router, &mut paths);
    router = add_root_redirect(router, &config, &mut paths);
    router.with_state(Arc::new(config))
}

#[must_use]
pub fn swagger_page_body(config: &DocsRouteConfig) -> Option<String> {
    let openapi_url = config.urls.openapi_url.as_deref()?;
    let prefix = config.server_root_path.trim_end_matches('/');
    Some(swagger_ui_page(&SwaggerPageInputs {
        title: format!("{} - Swagger UI", config.title),
        openapi_url: format!("{prefix}{openapi_url}"),
        oauth2_redirect_url: Some(format!("{prefix}/docs/oauth2-redirect")),
        swagger_base: swagger_base(&config.server_root_path),
    }))
}

#[must_use]
pub fn redoc_page_body(config: &DocsRouteConfig) -> Option<String> {
    let openapi_url = config.urls.openapi_url.as_deref()?;
    let prefix = config.server_root_path.trim_end_matches('/');
    Some(redoc_page(
        &format!("{} - ReDoc", config.title),
        &format!("{prefix}{openapi_url}"),
    ))
}

fn add_page_routes(
    mut router: Router<Arc<DocsRouteConfig>>,
    config: &DocsRouteConfig,
    paths: &mut BTreeSet<String>,
) -> Router<Arc<DocsRouteConfig>> {
    if config.urls.openapi_url.is_none() {
        return router;
    }
    router = add_swagger_routes(router, config, paths);
    let Some(url) = config.urls.redoc_url.as_deref() else {
        return router;
    };
    if paths.insert(url.to_owned()) {
        router = router.route(url, get(redoc_page_response).fallback(method_not_allowed));
    }
    router
}

fn add_swagger_routes(
    mut router: Router<Arc<DocsRouteConfig>>,
    config: &DocsRouteConfig,
    paths: &mut BTreeSet<String>,
) -> Router<Arc<DocsRouteConfig>> {
    let Some(docs_url) = config.urls.docs_url.as_deref() else {
        return router;
    };
    if paths.insert(docs_url.to_owned()) {
        router = router.route(
            docs_url,
            get(swagger_page_response).fallback(method_not_allowed),
        );
    }
    if paths.insert("/docs/oauth2-redirect".to_owned()) {
        router = router.route(
            "/docs/oauth2-redirect",
            get(|| async { Html(oauth2_redirect_page()) }).fallback(method_not_allowed),
        );
    }
    router
}

fn add_swagger_mount(
    mut router: Router<Arc<DocsRouteConfig>>,
    paths: &mut BTreeSet<String>,
) -> Router<Arc<DocsRouteConfig>> {
    if paths.insert("/swagger".to_owned()) {
        router = router.route("/swagger", any(swagger_slash_redirect));
    }
    if paths.insert("/swagger/".to_owned()) {
        router = router
            .route(
                "/swagger/",
                get(|| async { detail_response(StatusCode::NOT_FOUND, "Not Found") })
                    .fallback(method_not_allowed),
            )
            .route(
                "/swagger/{*path}",
                get(swagger_file).fallback(method_not_allowed),
            );
    }
    router
}

fn add_root_redirect(
    mut router: Router<Arc<DocsRouteConfig>>,
    config: &DocsRouteConfig,
    paths: &mut BTreeSet<String>,
) -> Router<Arc<DocsRouteConfig>> {
    if config.urls.docs_url.as_deref() == Some("/") {
        return router;
    }
    if config.urls.root_redirect_url.is_none() {
        return router;
    }
    if paths.insert("/".to_owned()) {
        router = router.route(
            "/",
            get(root_redirect_response).fallback(method_not_allowed),
        );
    }
    router
}

fn swagger_base(server_root_path: &str) -> String {
    if server_root_path.ends_with('/') {
        format!("{server_root_path}swagger")
    } else {
        format!("{server_root_path}/swagger")
    }
}

async fn swagger_page_response(State(config): State<Arc<DocsRouteConfig>>) -> Response {
    swagger_page_body(&config).map_or_else(
        || detail_response(StatusCode::NOT_FOUND, "Not Found"),
        |page| Html(page).into_response(),
    )
}

async fn redoc_page_response(State(config): State<Arc<DocsRouteConfig>>) -> Response {
    redoc_page_body(&config).map_or_else(
        || detail_response(StatusCode::NOT_FOUND, "Not Found"),
        |page| Html(page).into_response(),
    )
}

async fn root_redirect_response(State(config): State<Arc<DocsRouteConfig>>) -> Response {
    config.urls.root_redirect_url.as_ref().map_or_else(
        || detail_response(StatusCode::NOT_FOUND, "Not Found"),
        |url| Redirect::temporary(url).into_response(),
    )
}

async fn swagger_slash_redirect(headers: HeaderMap, OriginalUri(uri): OriginalUri) -> Response {
    let mut target = String::from("/swagger/");
    if let Some(query) = uri.query() {
        target.push('?');
        target.push_str(query);
    }
    let location = match headers
        .get(header::HOST)
        .and_then(|host| host.to_str().ok())
    {
        Some(host) => format!("http://{host}{target}"),
        None => target,
    };
    Redirect::temporary(&location).into_response()
}

async fn swagger_file(Path(path): Path<String>) -> Response {
    let Some(name) = asset_name(&path) else {
        return detail_response(StatusCode::NOT_FOUND, "Not Found");
    };
    let Some(asset) = swagger_asset(name) else {
        return detail_response(StatusCode::NOT_FOUND, "Not Found");
    };
    let content_type = match asset.content_type {
        "text/css" => "text/css; charset=utf-8",
        "text/javascript" => "text/javascript; charset=utf-8",
        other => other,
    };
    let mut response = Response::new(Body::from(asset.bytes));
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    response
}

async fn method_not_allowed() -> Response {
    let mut response = detail_response(StatusCode::METHOD_NOT_ALLOWED, "Method Not Allowed");
    response
        .headers_mut()
        .insert(header::ALLOW, HeaderValue::from_static("GET, HEAD"));
    response
}

fn detail_response(status: StatusCode, detail: &str) -> Response {
    let body = format!("{{\"detail\":\"{detail}\"}}");
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    response
}

// StaticFiles normalizes the mounted path with os.path.normpath before looking the file up
fn asset_name(path: &str) -> Option<&str> {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            name => parts.push(name),
        }
    }
    match parts.as_slice() {
        [name] => Some(name),
        _ => None,
    }
}
