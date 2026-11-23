pub mod app;
mod encoding;
mod float;
pub mod renderer;

mod tests;

use std::io::Write as _;

use flate2::Compression;
use flate2::write::GzEncoder;

use crate::collector_registry::CollectorRegistry;

use encoding::openmetrics::render_openmetrics;

// RESPONSE_CHUNK_SIZE_BYTES: the largest frame LiteLLM's StreamingResponse sends
pub const RESPONSE_CHUNK_SIZE_BYTES: usize = 65_536;

const TEXT_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";
const OPENMETRICS_CONTENT_TYPE: &str = "application/openmetrics-text; version=1.0.0; charset=utf-8";
const OPENMETRICS_MEDIA_TYPE: &str = "application/openmetrics-text";

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Encoder {
    Text,
    OpenMetrics,
}

impl Encoder {
    #[must_use]
    pub const fn content_type(self) -> &'static str {
        match self {
            Self::Text => TEXT_CONTENT_TYPE,
            Self::OpenMetrics => OPENMETRICS_CONTENT_TYPE,
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ScrapeRequest {
    pub encoder: Encoder,
    pub content_type: &'static str,
    pub gzipped: bool,
    pub metric_names: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ScrapeRenderError {
    #[error("the scrape render task stopped before it sent a body")]
    RenderTaskStopped,
}

// choose_encoder and gzip_accepted: the media type is compared before its parameters and case-sensitively, the encoding lowercased
#[must_use]
pub fn parse_scrape_request(
    accept: &str,
    accept_encoding: &str,
    metric_names: Vec<String>,
) -> ScrapeRequest {
    let encoder = if accept
        .split(',')
        .any(|part| media_type(part) == OPENMETRICS_MEDIA_TYPE)
    {
        Encoder::OpenMetrics
    } else {
        Encoder::Text
    };
    let gzipped = accept_encoding
        .split(',')
        .any(|part| media_type(part).to_lowercase() == "gzip");
    ScrapeRequest {
        encoder,
        content_type: encoder.content_type(),
        gzipped,
        metric_names,
    }
}

fn media_type(part: &str) -> &str {
    let without_parameters = part
        .split_once(';')
        .map_or(part, |(head, _parameters)| head);
    without_parameters.trim()
}

// LiteLLM's render_scrape: the encoder of the request over the whole registry or the restricted one, gzipped when asked
#[must_use]
pub fn render_scrape(registry: &CollectorRegistry, request: &ScrapeRequest) -> Vec<u8> {
    let rendered = match request.encoder {
        Encoder::Text => encoding::render_text(registry, &request.metric_names),
        Encoder::OpenMetrics => render_openmetrics(registry, &request.metric_names),
    };
    if request.gzipped {
        gzip(&rendered)
    } else {
        rendered
    }
}

fn gzip(body: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    if let Err(error) = encoder.write_all(body) {
        tracing::error!(%error, "gzip-compressing a scrape body failed");
        return Vec::new();
    }
    match encoder.finish() {
        Ok(compressed) => compressed,
        Err(error) => {
            tracing::error!(%error, "gzip-compressing a scrape body failed");
            Vec::new()
        }
    }
}
