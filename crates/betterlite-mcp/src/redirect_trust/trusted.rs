use betterlite_http::user_url::split::Parsed;
use tracing::warn;

use super::RedirectAllowlists;
use super::shape::{
    RedirectShape, RedirectUriRejected, is_loopback_redirect_host, parse_redirect_uri,
    validate_redirect_uri_shape,
};
use crate::oauth_resource::base_url::{RequestOrigin, get_request_base_url, strip_default_port};
use crate::oauth_resource::python_url;

const TRUSTED_REDIRECT_HINT: &str = "Align the proxy public URL with the browser URL. Set PROXY_BASE_URL to your HTTPS origin (e.g. https://litellm.example.com), or enable general_settings.use_x_forwarded_for with mcp_trusted_proxy_ranges for your ingress. If the redirect_uri is a legitimate separate-origin OAuth client (e.g. a web app registering with the proxy from another host via dynamic client registration), add its origin to MCP_TRUSTED_REDIRECT_ORIGINS. Verify: curl https://<host>/.well-known/oauth-authorization-server | jq .issuer \u{2014} issuer must match window.location.origin in the UI.";

// LiteLLM's validate_trusted_redirect_uri: native callbacks and http(s) hygiene first, then the origin checks.
pub fn validate_trusted_redirect_uri(
    origin: &RequestOrigin<'_>,
    proxy_base_url: &str,
    redirect_uri: &str,
    allowlists: &RedirectAllowlists,
) -> Result<(), RedirectUriRejected> {
    let parsed = parse_redirect_uri(redirect_uri)?;
    if validate_redirect_uri_shape(&parsed, &allowlists.native_callbacks)?
        == RedirectShape::NativeCallback
    {
        return Ok(());
    }
    let redirect_netloc = strip_default_port(&parsed.scheme, &parsed.netloc);
    let proxy_base = match get_request_base_url(proxy_base_url, origin) {
        Ok(base) => Some(base),
        Err(error) => {
            warn!(
                error = %error,
                "validate_trusted_redirect_uri: could not determine the proxy origin, falling back to loopback + allowlist"
            );
            None
        }
    };
    if trusted_redirect_uri_allowed(&parsed, &redirect_netloc, proxy_base.as_deref(), allowlists) {
        return Ok(());
    }
    let rejection = trusted_rejection(
        redirect_uri,
        &parsed,
        &redirect_netloc,
        proxy_base.as_deref(),
    );
    warn!(
        redirect_uri = %redirect_uri,
        description = %rejection.description,
        proxy_base = ?proxy_base,
        proxy_base_url = %proxy_base_url,
        x_forwarded_proto = ?origin.x_forwarded_proto,
        x_forwarded_host = ?origin.x_forwarded_host,
        x_forwarded_port = ?origin.x_forwarded_port,
        trusted_redirect_origins = ?allowlists.origins,
        trusted_native_redirect_uris = ?allowlists.native_callbacks,
        "MCP OAuth: rejecting redirect_uri"
    );
    Err(rejection)
}

// LiteLLM's _trusted_redirect_uri_is_allowed: same-origin, any loopback host, or an allowlisted https origin.
fn trusted_redirect_uri_allowed(
    parsed: &Parsed,
    redirect_netloc: &str,
    proxy_base: Option<&str>,
    allowlists: &RedirectAllowlists,
) -> bool {
    if let Some(base) = proxy_base
        && let Ok(proxy) = python_url::parse(base)
    {
        let proxy_netloc = strip_default_port(&proxy.scheme, &proxy.netloc);
        if parsed.scheme == proxy.scheme && proxy_netloc.as_str() == redirect_netloc {
            return true;
        }
    }
    if is_loopback_redirect_host(parsed) {
        return true;
    }
    parsed.scheme == "https"
        && allowlists
            .origins
            .iter()
            .any(|entry| matches_trusted_origin_entry(redirect_netloc, entry))
}

// LiteLLM's _matches_trusted_origin_entry: an exact host[:port], or *.suffix for a strictly deeper subdomain.
#[must_use]
pub fn matches_trusted_origin_entry(netloc: &str, entry: &str) -> bool {
    let Some(suffix) = entry.strip_prefix("*.") else {
        return netloc == entry;
    };
    if suffix.is_empty() || suffix.starts_with('.') {
        return false;
    }
    let host = netloc.split_once(':').map_or(netloc, |(host, _)| host);
    host != suffix && host.ends_with(&format!(".{suffix}"))
}

// LiteLLM's _build_trusted_redirect_rejection_message: the client text hides the proxy origin.
fn trusted_rejection(
    redirect_uri: &str,
    parsed: &Parsed,
    redirect_netloc: &str,
    proxy_base: Option<&str>,
) -> RedirectUriRejected {
    let redirect_origin = format!("{}://{redirect_netloc}", parsed.scheme);
    let proxy = proxy_base.and_then(|base| python_url::parse(base).ok());
    let mut mismatch_parts: Vec<String> = Vec::new();
    if let Some(proxy) = &proxy
        && !proxy.netloc.is_empty()
    {
        if parsed.scheme != proxy.scheme {
            mismatch_parts.push(format!(
                "scheme: redirect_uri uses '{}', but the proxy resolved a different scheme (TLS often terminates at ingress \u{2014} set PROXY_BASE_URL to https://\u{2026} or trust X-Forwarded-Proto from your ingress)",
                parsed.scheme
            ));
        }
        let proxy_netloc = strip_default_port(&proxy.scheme, &proxy.netloc);
        if proxy_netloc.as_str() != redirect_netloc {
            mismatch_parts.push(format!(
                "host/port: redirect_uri '{redirect_netloc}' does not match the proxy origin"
            ));
        }
    }
    let description = if mismatch_parts.is_empty() {
        format!(
            "redirect_uri ('{redirect_uri}') is not allowed: not same-origin with the proxy origin, not loopback, and not listed in MCP_TRUSTED_REDIRECT_ORIGINS."
        )
    } else {
        format!(
            "redirect_uri origin ({redirect_origin}) does not match the proxy origin. {}",
            mismatch_parts.join("; ")
        )
    };
    RedirectUriRejected {
        description,
        hint: Some(TRUSTED_REDIRECT_HINT.to_owned()),
        redirect_uri: Some(redirect_uri.to_owned()),
    }
}
