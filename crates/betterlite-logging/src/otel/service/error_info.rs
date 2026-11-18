use crate::fields::payload::records::ErrorInformation;
use crate::logging_metadata::error_info::ExceptionLogging;
use crate::logging_metadata::exception::ExceptionFacts;
use crate::otel::legacy_policy::error_recording::RecordedException;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::v2_callback::span_error::ErrorInformationConfig;
use crate::payload::compose::error_info::expected_client_error;

// litellm.litellm_core_utils.litellm_logging.StandardLoggingPayloadSetup.get_error_information
pub(super) fn logging_context<'config>(
    config: &'config ErrorInformationConfig,
    exception: Option<&'config ExceptionFacts>,
) -> ExceptionLogging<'config> {
    ExceptionLogging {
        exception,
        log_client_error_tracebacks: config.log_client_error_tracebacks,
        expected_client_error: expected_client_error(exception),
        redactor: config.redactor.as_deref(),
        normalizer: &config.normalizer,
    }
}

// what span.record_exception reads off the exception object
#[must_use]
pub(super) fn recorded_exception(exception: Option<&ExceptionFacts>) -> Option<RecordedException> {
    let facts = exception?;
    Some(RecordedException {
        type_name: facts.class_name.clone(),
        message: facts.message.clone().unwrap_or_default(),
        stacktrace: facts.traceback_lines.concat(),
    })
}

// kwargs={"standard_logging_object": {"error_information": error_information}}
#[must_use]
pub(super) fn error_payload(information: &ErrorInformation) -> MetadataMap {
    let fields: MetadataMap = [
        ("error_code", information.error_code.clone().flatten()),
        ("error_class", information.error_class.clone().flatten()),
        ("llm_provider", information.llm_provider.clone().flatten()),
        ("traceback", information.traceback.clone().flatten()),
        ("error_message", information.error_message.clone().flatten()),
    ]
    .into_iter()
    .filter_map(|(key, value)| value.map(|text| (key.to_owned(), OtelModelValue::String(text))))
    .collect();
    MetadataMap::from([(
        "error_information".to_owned(),
        OtelModelValue::Object(fields),
    )])
}

// the pinned code writes str(exception) into the exception attributes
#[must_use]
pub(super) fn exception_text(exception: Option<&ExceptionFacts>) -> String {
    exception.map_or_else(
        || "None".to_owned(),
        |facts| facts.message.clone().unwrap_or_default(),
    )
}
