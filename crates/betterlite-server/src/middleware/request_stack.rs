mod tests;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::middleware::from_fn;
use betterlite_config::sections::request_size_limit::Settings as SizeSettings;
use betterlite_config::sections::security_headers::Settings as HeadersSettings;

use crate::middleware::gzip_buffered::GzipBufferedLayer;
use crate::middleware::redis_request_batch::RedisBatchLayer;
use crate::middleware::request_root_paths::redirect_root_slash;
use crate::middleware::request_size_limit::SizeLimitLayer;
use crate::middleware::security_headers::SecurityHeadersLayer;

// LiteLLM adds the size limit after gzip and the security headers, so it is outermost and its 413 is bare
pub fn wrap(application: Router, size_limit: &SizeSettings, security: &HeadersSettings) -> Router {
    let routed = application
        .layer(from_fn(redirect_root_slash))
        .layer(RedisBatchLayer)
        .layer(SecurityHeadersLayer {
            enable_hsts: security.enable_hsts,
        })
        .layer(GzipBufferedLayer);
    match SizeLimitLayer::from_settings(size_limit) {
        Some(limit) => {
            // extractor limits count in usize, and the layer's own count is the same number
            let body_limit = usize::try_from(limit.max_bytes).unwrap_or(usize::MAX);
            routed.layer(DefaultBodyLimit::max(body_limit)).layer(limit)
        }
        None => routed,
    }
}
