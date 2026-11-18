use crate::logging_metadata::error_info::get_error_information;
use crate::logging_metadata::exception::ExceptionFacts;
use crate::otel::legacy_policy::error_recording::record_exception_on_span;
use crate::otel::v2_callback::live_span::LiveSpan;
use crate::otel::v2_callback::span_error::ErrorInformationConfig;

use super::error_info;

pub fn record_error_attributes(
    span: Option<&LiveSpan>,
    exception: Option<&ExceptionFacts>,
    status_code: i64,
    info: &ErrorInformationConfig,
) {
    let Some(span) = span else {
        return;
    };
    if exception.is_none() {
        return;
    }
    let mut information =
        get_error_information(&error_info::logging_context(info, exception), None);
    information.error_code = Some(Some(status_code.to_string()));
    // no exception key in the kwargs, so no exception event
    record_exception_on_span(
        &mut *span.lock(),
        None,
        &error_info::error_payload(&information),
    );
}
