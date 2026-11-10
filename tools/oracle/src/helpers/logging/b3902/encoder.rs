use std::sync::Arc;

use betterlite_logging::langfuse_exporter::encoder::SpanEncoder;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::SpanData;

use super::UNENCODABLE_KEY;
use super::journal::Journal;

#[derive(Debug)]
pub(crate) struct RecordingEncoder {
    pub(crate) journal: Arc<Journal>,
}

impl SpanEncoder for RecordingEncoder {
    fn encode(&self, spans: &[SpanData], _resource: &Resource) -> Option<Vec<u8>> {
        let rejected = spans.iter().any(|span| {
            span.attributes
                .iter()
                .any(|attribute| attribute.key.as_str() == UNENCODABLE_KEY)
        });
        if rejected {
            return None;
        }
        let index = self.journal.record_encode(spans);
        Some(index.to_be_bytes().to_vec())
    }
}
