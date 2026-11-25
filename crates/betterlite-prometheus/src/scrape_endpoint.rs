pub mod app;
mod encoding;
pub(crate) mod float;
pub mod renderer;

mod tests;

use std::io::Write as _;
use std::sync::Arc;

use flate2::Compression;
use flate2::write::GzEncoder;

use crate::collector_registry::CollectorRegistry;
use crate::multiprocess::MultiprocessError;
use crate::multiprocess::collector::MultiProcessCollector;

use encoding::openmetrics::render_openmetrics;

// RESPONSE_CHUNK_SIZE_BYTES: the largest frame LiteLLM's StreamingResponse sends
pub const RESPONSE_CHUNK_SIZE_BYTES: usize = 0x0001_0000;

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

// what prometheus_client's collect yields and the encoders write: a whole family, its samples already named and valued
#[derive(Clone, Debug, PartialEq)]
pub struct MetricFamily {
    pub name: String,
    pub help: String,
    pub kind: MetricKind,
    pub samples: Vec<MetricSample>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MetricSample {
    pub name: String,
    pub labels: Vec<(String, String)>,
    pub value: f64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetricKind {
    Counter,
    Gauge,
    Summary,
    Histogram,
}

impl MetricKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Counter => "counter",
            Self::Gauge => "gauge",
            Self::Summary => "summary",
            Self::Histogram => "histogram",
        }
    }

    // metrics_core.METRIC_TYPES as the four types prometheus_client's multiprocess writers create
    #[must_use]
    pub fn from_type_name(name: &str) -> Option<Self> {
        match name {
            "counter" => Some(Self::Counter),
            "gauge" => Some(Self::Gauge),
            "summary" => Some(Self::Summary),
            "histogram" => Some(Self::Histogram),
            _other => None,
        }
    }
}

// what a scrape renders: the in-process registry, or the folder the worker processes write into
#[derive(Debug)]
pub enum ScrapeSource {
    Registry(Arc<CollectorRegistry>),
    Multiprocess(MultiProcessCollector),
}

impl ScrapeSource {
    // LiteLLM reads the folder again at every scrape, so the collect happens inside the render
    pub fn render(&self, request: &ScrapeRequest) -> Result<Vec<u8>, ScrapeRenderError> {
        match self {
            Self::Registry(registry) => Ok(render_scrape(registry, request)),
            Self::Multiprocess(collector) => {
                let families = collector
                    .collect()
                    .map_err(|error| ScrapeRenderError::Multiprocess(Arc::new(error)))?;
                Ok(render_families(&families, request))
            }
        }
    }
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum ScrapeRenderError {
    #[error("the scrape render task stopped before it sent a body")]
    RenderTaskStopped,
    #[error("the multiprocess collector failed")]
    Multiprocess(#[source] Arc<MultiprocessError>),
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
    let metrics = registry.metrics();
    let families = encoding::families_of(&metrics);
    render_families(&families, request)
}

// the family-level render both ScrapeSource arms encode through
#[must_use]
pub fn render_families(families: &[MetricFamily], request: &ScrapeRequest) -> Vec<u8> {
    let mut rendered = match request.encoder {
        Encoder::Text => encoding::render_text(families, &request.metric_names),
        Encoder::OpenMetrics => render_openmetrics(families, &request.metric_names),
    };
    if request.gzipped {
        rendered = gzip(&rendered);
    }
    rendered
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
