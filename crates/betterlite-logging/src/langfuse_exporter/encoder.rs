use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use opentelemetry_proto::transform::common::tonic::ResourceAttributesWithSchema;
// ast-grep-ignore: no-long-names
use opentelemetry_proto::transform::trace::tonic::group_spans_by_resource_and_scope;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::SpanData;
use prost::Message as _;
use std::fmt::Debug;

pub trait SpanEncoder: Send + Sync + Debug {
    // None is the encoder refusing the spans, which the exporter answers by encoding each span alone
    fn encode(&self, spans: &[SpanData], resource: &Resource) -> Option<Vec<u8>>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct OtlpSpanEncoder;

impl SpanEncoder for OtlpSpanEncoder {
    fn encode(&self, spans: &[SpanData], resource: &Resource) -> Option<Vec<u8>> {
        let attributes = ResourceAttributesWithSchema::from(resource);
        // ast-grep-ignore: no-long-names
        let resource_spans = group_spans_by_resource_and_scope(spans.to_vec(), &attributes);
        let request = ExportTraceServiceRequest { resource_spans };
        Some(request.encode_to_vec())
    }
}
