use betterlite_http::proxy::transport_error::TransportError;

use super::ChatError;

pub(super) fn http_failure(source: TransportError) -> ChatError {
    match source {
        TransportError::ConnectTimeout(_) | TransportError::Deadline(_) => ChatError::Timeout,
        TransportError::BodyTooLarge { .. } | TransportError::HeadersTooLarge { .. } => {
            ChatError::TooLarge
        }
        TransportError::Rejected(_)
        | TransportError::Build(_)
        | TransportError::Connect(_)
        | TransportError::Request(_)
        | TransportError::Body(_)
        | TransportError::InvalidHeader { .. }
        | TransportError::MultipartContentType { .. }
        | TransportError::MultipartBoundaryHeader { .. }
        | TransportError::MultipartBoundary { .. }
        | TransportError::Mock(_)
        | TransportError::Closed
        | TransportError::DrainTimeout { .. }
        | TransportError::ConfiguredUrl { .. }
        | TransportError::TooManyRedirects { .. }
        | TransportError::RedirectTarget { .. }
        | TransportError::RedirectUrl { .. }
        | TransportError::RedirectUserinfo
        | TransportError::RedirectScheme { .. }
        | TransportError::InsecureRedirect
        | TransportError::RedirectBody => ChatError::Http { source },
    }
}
