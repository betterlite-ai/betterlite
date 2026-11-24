use std::convert::Infallible;
use std::sync::Arc;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::Response;
use futures_util::stream;

use super::renderer::CoalescedScrapeRenderer;
use super::{RESPONSE_CHUNK_SIZE_BYTES, ScrapeSource, parse_scrape_request};

// LiteLLM's make_metrics_asgi_app: every method and path answers a scrape, over the registry or the multiprocess folder the caller names
pub fn make_metrics_app(source: ScrapeSource) -> Router {
    let renderer = Arc::new(CoalescedScrapeRenderer::new(source));
    let router = Router::new().fallback(scrape);
    router.with_state(renderer)
}

pub(crate) async fn scrape(
    State(renderer): State<Arc<CoalescedScrapeRenderer>>,
    headers: HeaderMap,
    Query(parameters): Query<Vec<(String, String)>>,
) -> Response {
    let accept = headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let accept_encoding = headers
        .get(header::ACCEPT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let metric_names: Vec<String> = parameters
        .into_iter()
        .filter(|(name, _value)| name == "name[]")
        .map(|(_name, value)| value)
        .collect();
    let request = parse_scrape_request(accept, accept_encoding, metric_names);
    let content_type = request.content_type;
    let gzipped = request.gzipped;
    match renderer.render(request).await {
        Ok(body) => response_with(body, content_type, gzipped),
        Err(_error) => {
            let mut response = Response::new(Body::empty());
            *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
            response
        }
    }
}

fn response_with(body: Bytes, content_type: &'static str, gzipped: bool) -> Response {
    let stream = stream::unfold(Some(body), next_frame);
    let mut response = Response::new(Body::from_stream(stream));
    let headers = response.headers_mut();
    let _content_type =
        headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    if gzipped {
        let _content_encoding =
            headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
    }
    response
}

// _chunks: the body leaves in frames of at most RESPONSE_CHUNK_SIZE_BYTES, with no length the response could announce
async fn next_frame(state: Option<Bytes>) -> Option<(Result<Bytes, Infallible>, Option<Bytes>)> {
    let mut frame = state?;
    if frame.is_empty() {
        return None;
    }
    let take = frame.len().min(RESPONSE_CHUNK_SIZE_BYTES);
    let rest = frame.split_off(take);
    let next = (!rest.is_empty()).then_some(rest);
    Some((Ok(frame), next))
}
