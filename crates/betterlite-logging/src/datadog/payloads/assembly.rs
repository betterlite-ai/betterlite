use crate::datadog::apm::Tracer;
use crate::datadog::tagging::Environment;

use super::status::DataDogStatus;
use super::trace::add_trace_context;
use super::types::DatadogPayload;

// Both payload builders assemble the envelope the same way; only the message and the tags differ
pub(crate) fn assemble_payload(
    environment: &Environment,
    status: DataDogStatus,
    tags: &[String],
    message: String,
    tracer: &dyn Tracer,
) -> DatadogPayload {
    let mut payload = DatadogPayload {
        ddsource: environment.source.clone(),
        ddtags: tags.join(","),
        hostname: environment.hostname.clone(),
        message,
        service: environment.service.clone(),
        status,
        trace_id: None,
        span_id: None,
    };
    add_trace_context(&mut payload, tracer);
    payload
}
