use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_core::api::rerank_provider_plan::plan::{DIAGNOSTIC_PREFIX, RerankException};
use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;
use betterlite_logging::logging_pre_post::env::{LogRecord, LogSink};
use betterlite_logging::logging_state::LoggingInit;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_redact::secret::Redactor;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

use super::super::error::SdkHttpError;
use super::super::result::SyncDiagnostic;
use super::{FIXTURE_INIT, Failure};

const TARGET: &str = "betterlite_core::api::rerank_provider_plan";

pub(super) struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

pub(super) fn logging_state(redactor: &Redactor) -> Result<LoggingState, SdkHttpError> {
    let init: LoggingInit = serde_json::from_str(FIXTURE_INIT).map_err(SdkHttpError::Serialize)?;
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("generated"),
        Duration::default(),
    )
    .map_err(SdkHttpError::LoggingState)
}

// The exception the SDK raises for this class, as the pinned litellm.exceptions module names it
pub(super) fn failure(exception: &RerankException) -> Result<Failure, SdkHttpError> {
    let metadata = exception.class.metadata();
    Ok(Failure {
        class: class_name(&exception.class)?.to_owned(),
        status: Some(metadata.status_code),
        kind: text_field(metadata.error_type),
        param: text_field(metadata.param),
        code: metadata.openai_code,
        message: exception.message.clone(),
    })
}

fn text_field(field: WireField) -> Option<String> {
    match field {
        WireField::Text(text) => Some(text),
        WireField::Missing(_) | WireField::Null => None,
    }
}

fn class_name(class: &ErrorClass) -> Result<&'static str, SdkHttpError> {
    match class {
        ErrorClass::Authentication(_) => Ok("AuthenticationError"),
        ErrorClass::NotFound(_) => Ok("NotFoundError"),
        ErrorClass::PermissionDenied(_) => Ok("PermissionDeniedError"),
        ErrorClass::BadRequest(_) => Ok("BadRequestError"),
        ErrorClass::UnprocessableEntity(_) => Ok("UnprocessableEntityError"),
        ErrorClass::ContextWindowExceeded(_) => Ok("ContextWindowExceededError"),
        ErrorClass::ContentPolicyViolation(_) => Ok("ContentPolicyViolationError"),
        ErrorClass::ServiceUnavailable(_) => Ok("ServiceUnavailableError"),
        ErrorClass::BadGateway(_) => Ok("BadGatewayError"),
        ErrorClass::InternalServer(_) => Ok("InternalServerError"),
        ErrorClass::Timeout(_) => Ok("Timeout"),
        ErrorClass::RateLimit(_) => Ok("RateLimitError"),
        ErrorClass::Api(_) => Ok("APIError"),
        ErrorClass::ApiConnection(_) => Ok("APIConnectionError"),
        ErrorClass::UnsupportedParams(_) => Ok("UnsupportedParamsError"),
        ErrorClass::BudgetExceeded(_)
        | ErrorClass::ApiResponseValidation(_)
        | ErrorClass::JsonSchemaValidation(_)
        | ErrorClass::GuardrailRaised(_)
        | ErrorClass::MidStreamFallback(_)
        | ErrorClass::Unclassified(_) => Err(SdkHttpError::Unprojected(format!("{class:?}"))),
    }
}

#[derive(Clone, Default)]
pub(super) struct Diagnostics(Arc<Mutex<Vec<SyncDiagnostic>>>);

impl Diagnostics {
    pub(super) fn take(&self) -> Vec<SyncDiagnostic> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

#[derive(Default)]
struct MessageVisitor(Option<String>);

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = Some(format!("{value:?}"));
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Diagnostics {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if !event.metadata().target().starts_with(TARGET) {
            return;
        }
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        let Some(message) = visitor.0 else {
            return;
        };
        if !message.starts_with(DIAGNOSTIC_PREFIX) {
            return;
        }
        let level = match *event.metadata().level() {
            Level::ERROR => "error",
            Level::WARN => "warning",
            Level::INFO => "info",
            Level::DEBUG => "debug",
            Level::TRACE => "trace",
        };
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(SyncDiagnostic {
                level: level.to_owned(),
                prefix: DIAGNOSTIC_PREFIX.to_owned(),
            });
    }
}
