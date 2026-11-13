use std::collections::BTreeSet;
use std::mem::take;

use axum::Router;
use axum::body::Body;
use axum::extract::Path;
use axum::extract::rejection::PathRejection;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse as _, Redirect, Response};
use axum::routing::{MethodFilter, MethodRouter, any, get, on};
use betterlite_openapi::docs_pages::{
    SwaggerPageInputs, oauth2_redirect_page, redoc_page, swagger_asset, swagger_ui_page,
};
use betterlite_openapi::docs_settings::DocsUrls;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use responses::{detail_response, method_not_allowed, root_method_not_allowed, slash_redirect};

mod responses;
mod tests;

pub struct DocsRouteConfig {
    pub urls: DocsUrls,
    pub title: String,
    pub server_root_path: String,
}

#[derive(Clone, Copy)]
pub enum DocsPage {
    Swagger,
    Redoc,
}

// Starlette's RedirectResponse quote() safe set, plus the unreserved characters quote always keeps
const REDIRECT_SAFE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~')
    .remove(b':')
    .remove(b'/')
    .remove(b'%')
    .remove(b'#')
    .remove(b'?')
    .remove(b'=')
    .remove(b'@')
    .remove(b'[')
    .remove(b']')
    .remove(b'!')
    .remove(b'$')
    .remove(b'&')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')')
    .remove(b'*')
    .remove(b'+')
    .remove(b',')
    .remove(b';');

pub fn docs_router(config: DocsRouteConfig) -> Router {
    // the caller registers the OpenAPI route, and FastAPI adds it before these pages
    let mut builder = RouteBuilder::new(config);
    add_page_routes(&mut builder);
    add_mount_routes(&mut builder);
    add_root_redirect(&mut builder);
    builder.finish()
}

#[must_use]
pub fn page_body(config: &DocsRouteConfig, openapi_url: &str, page: DocsPage) -> String {
    let prefix = config.server_root_path.trim_end_matches('/');
    match page {
        DocsPage::Swagger => swagger_ui_page(&SwaggerPageInputs {
            title: format!("{} - Swagger UI", config.title),
            openapi_url: format!("{prefix}{openapi_url}"),
            oauth2_redirect_url: Some(format!("{prefix}/docs/oauth2-redirect")),
            swagger_base: swagger_base(&config.server_root_path),
        }),
        DocsPage::Redoc => redoc_page(
            &format!("{} - ReDoc", config.title),
            &format!("{prefix}{openapi_url}"),
        ),
    }
}

// Starlette quotes redirect urls with quote(url, safe=":/%#?=@[]!$&'()*+,;") and percent-encodes the rest
#[must_use]
pub fn redirect_location(host: Option<&str>, target: &str) -> String {
    let location = host.map_or_else(
        || target.to_owned(),
        |origin| format!("http://{origin}{target}"),
    );
    utf8_percent_encode(&location, REDIRECT_SAFE).to_string()
}

struct RouteBuilder {
    config: DocsRouteConfig,
    router: Router,
    paths: BTreeSet<String>,
    exact: Vec<String>,
}

impl RouteBuilder {
    fn new(config: DocsRouteConfig) -> Self {
        let paths = config.urls.openapi_url.iter().cloned().collect();
        Self {
            config,
            // Starlette reads a colon or asterisk at the start of a segment as a literal character
            router: Router::new().without_v07_checks(),
            paths,
            exact: Vec::new(),
        }
    }

    fn add(&mut self, path: &str, route: MethodRouter) {
        if self.paths.insert(path.to_owned()) {
            self.exact.push(path.to_owned());
            let router = take(&mut self.router);
            self.router = router.route(path, route);
        }
    }

    // FastAPI's redirect_slashes answers a route's other slash form with a 307 once nothing matched
    fn finish(mut self) -> Router {
        let exact = take(&mut self.exact);
        for path in exact {
            let toggled = match path.strip_suffix('/') {
                Some(stripped) => stripped.to_owned(),
                None if path.contains("{*") => continue,
                None => format!("{path}/"),
            };
            // Starlette skips the redirect for "/" and an empty path would not be a valid route
            if !toggled.starts_with('/') || toggled == "/" {
                continue;
            }
            if self.paths.insert(toggled.clone()) {
                let router = take(&mut self.router);
                self.router = router.route(&toggled, any(slash_redirect));
            }
        }
        self.router
    }
}

fn add_page_routes(builder: &mut RouteBuilder) {
    let Some(openapi_url) = builder.config.urls.openapi_url.clone() else {
        return;
    };
    if let Some(docs_url) = builder.config.urls.docs_url.clone() {
        let route = page_router(&builder.config, &openapi_url, DocsPage::Swagger);
        builder.add(&docs_url, route);
        builder.add(
            "/docs/oauth2-redirect",
            get(|| async { Html(oauth2_redirect_page()) }).fallback(method_not_allowed),
        );
    }
    if let Some(redoc_url) = builder.config.urls.redoc_url.clone() {
        let route = page_router(&builder.config, &openapi_url, DocsPage::Redoc);
        builder.add(&redoc_url, route);
    }
}

fn add_mount_routes(builder: &mut RouteBuilder) {
    builder.add(
        "/swagger/",
        get(|| async { detail_response(StatusCode::NOT_FOUND, "Not Found") })
            .fallback(method_not_allowed),
    );
    builder.add(
        "/swagger/{*path}",
        get(swagger_file).fallback(method_not_allowed),
    );
}

fn add_root_redirect(builder: &mut RouteBuilder) {
    if builder.config.urls.docs_url.as_deref() == Some("/") {
        return;
    }
    let Some(url) = builder.config.urls.root_redirect_url.clone() else {
        return;
    };
    let route = redirect_router(redirect_location(None, &url));
    builder.add("/", route);
}

fn redirect_router(location: String) -> MethodRouter {
    on(MethodFilter::GET, move || {
        let target = location.clone();
        async move { Redirect::temporary(&target).into_response() }
    })
    // axum routes HEAD to a GET handler, so the HEAD endpoint must reject it explicitly
    .on(MethodFilter::HEAD, root_method_not_allowed)
    .fallback(root_method_not_allowed)
}

fn page_router(config: &DocsRouteConfig, openapi_url: &str, page: DocsPage) -> MethodRouter {
    let body = page_body(config, openapi_url, page);
    get(move || {
        let content = body.clone();
        async move { Html(content) }
    })
    .fallback(method_not_allowed)
}

fn swagger_base(server_root_path: &str) -> String {
    if server_root_path.ends_with('/') {
        format!("{server_root_path}swagger")
    } else {
        format!("{server_root_path}/swagger")
    }
}

async fn swagger_file(captured: Result<Path<String>, PathRejection>) -> Response {
    let Ok(Path(path)) = captured else {
        return detail_response(StatusCode::NOT_FOUND, "Not Found");
    };
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
