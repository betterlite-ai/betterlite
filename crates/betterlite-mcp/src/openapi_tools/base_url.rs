use tracing::info;

use crate::openapi_tools::document::OpenApiDocument;

const SPEC_SUFFIXES: [&str; 4] = [
    "/openapi.json",
    "/openapi.yaml",
    "/swagger.json",
    "/swagger.yaml",
];
const FILE_SUFFIXES: [&str; 3] = [".json", ".yaml", ".yml"];

// LiteLLM's get_base_url: servers[0].url, then the Swagger 2 host form, then the spec location itself
#[must_use]
pub fn get_base_url(spec: &OpenApiDocument, spec_path: Option<&str>) -> String {
    if let Some(server) = spec.servers.first() {
        let server_url = server.url.as_deref().unwrap_or_default();
        return relative_server_url(server_url, spec_path);
    }
    if let Some(host) = spec.host.as_deref() {
        return swagger_base(host, &spec.schemes, spec.base_path.as_deref());
    }
    match spec_path {
        Some(path) if path.starts_with("http://") || path.starts_with("https://") => {
            derived_base(path)
        }
        Some(_) | None => String::new(),
    }
}

fn relative_server_url(server_url: &str, spec_path: Option<&str>) -> String {
    let Some(path) = spec_path else {
        return server_url.to_owned();
    };
    let relative = server_url.starts_with('/');
    let http = path.starts_with("http://") || path.starts_with("https://");
    if !relative || !http {
        return server_url.to_owned();
    }
    let Some((scheme, rest)) = path.split_once("://") else {
        return server_url.to_owned();
    };
    let netloc = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let full = format!("{scheme}://{netloc}{server_url}");
    info!(
        "OpenAPI spec has relative server URL {server_url:?}; deriving base from spec_path: {full}"
    );
    full
}

fn swagger_base(host: &str, schemes: &[String], base_path: Option<&str>) -> String {
    let scheme = schemes.first().map_or("https", String::as_str);
    let base_path = base_path.unwrap_or_default();
    format!("{scheme}://{host}{base_path}")
}

fn derived_base(spec_path: &str) -> String {
    for suffix in SPEC_SUFFIXES {
        if let Some(base) = spec_path.strip_suffix(suffix) {
            info!("No server info in OpenAPI spec. Using derived base URL: {base}");
            return base.to_owned();
        }
    }
    let Some((base, last)) = spec_path.rsplit_once('/') else {
        return String::new();
    };
    if !FILE_SUFFIXES.iter().any(|suffix| last.ends_with(suffix)) {
        return String::new();
    }
    info!("No server info in OpenAPI spec. Using derived base URL: {base}");
    base.to_owned()
}
