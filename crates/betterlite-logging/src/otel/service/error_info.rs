use betterlite_models::call_helpers::expected_error::{ClientErrorFacts, is_expected_client_error};
use betterlite_models::decisions::Json;
use serde_json::Number;

use crate::fields::payload::records::ErrorInformation;
use crate::logging_metadata::error_info::ExceptionLogging;
use crate::logging_metadata::exception::ExceptionFacts;
use crate::otel::legacy_policy::error_recording::RecordedException;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::v2_callback::span_error::ErrorInformationConfig;

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

// the proxy's own 4xx is an expected client error; ExceptionFacts carries no HTTPException marker
fn expected_client_error(exception: Option<&ExceptionFacts>) -> bool {
    let Some(facts) = exception else {
        return false;
    };
    is_expected_client_error(Some(&ClientErrorFacts {
        code: facts.code.clone().map(Json::Text),
        status_code: facts
            .status_code
            .map(|status| Json::Number(Number::from(status))),
        llm_provider: facts.llm_provider.clone(),
        category: facts.category.clone(),
        http_exception: false,
        base_llm_exception: false,
    }))
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
