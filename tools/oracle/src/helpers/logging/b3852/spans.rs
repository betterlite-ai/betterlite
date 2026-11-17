use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use opentelemetry::trace::SpanId;
use opentelemetry_sdk::trace::{InMemorySpanExporter, SpanData};
use serde_json::json;

fn finished(sinks: &[InMemorySpanExporter]) -> Result<Vec<SpanData>, Box<HelperError>> {
    let mut spans = Vec::new();
    for sink in sinks {
        spans.extend(sink.get_finished_spans().or_value_error()?);
    }
    Ok(spans)
}

pub(super) fn span_records(sinks: &[InMemorySpanExporter]) -> Result<Vec<Json>, Box<HelperError>> {
    let mut records = Vec::new();
    for span in finished(sinks)? {
        records.push(json!({
            "name": span.name,
            "rooted": span.parent_span_id == SpanId::INVALID,
            "links": span.links.links.len(),
        }));
    }
    Ok(records)
}

pub(super) fn span_names(sinks: &[InMemorySpanExporter]) -> Result<Vec<String>, Box<HelperError>> {
    let mut names = Vec::new();
    for span in finished(sinks)? {
        names.push(span.name.to_string());
    }
    names.sort();
    Ok(names)
}

pub(super) fn shutdowns(sinks: &[InMemorySpanExporter]) -> usize {
    let mut count = 0;
    for sink in sinks {
        if sink.is_shutdown_called() {
            count = count.saturating_add(1);
        }
    }
    count
}
