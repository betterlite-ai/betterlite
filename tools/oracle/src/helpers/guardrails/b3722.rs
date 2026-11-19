use std::time::{Duration, SystemTime};

use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_guardrails::bedrock_decisions::assessments::{
    blocked_assessments, should_raise_blocked, violation_category_names,
};
use betterlite_guardrails::bedrock_decisions::exceptions::{
    BlockedGuardrail, body_has_exception, http_exception_for_blocked_guardrail,
    payload_has_exception, response_status,
};
use betterlite_guardrails::bedrock_decisions::logging::{
    AttemptLog, CallLog, log_attempt, log_failure, log_success,
};
use betterlite_guardrails::bedrock_decisions::tracing::{Pricing, tracing_detail};
use betterlite_guardrails::guardrail_base::GuardrailBase;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::settings::GuardrailSettings;
use betterlite_guardrails::guardrail_records::error::GuardrailSpanError;
use betterlite_guardrails::guardrail_records::record::{
    GuardrailInvocationRecord, GuardrailRecordContext, GuardrailSpanSink,
};
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Serialize;
use serde_json::to_value;
use wire::{
    Command, FailureView, GuardrailSetup, HttpExceptionArgs, Input, LogAttemptArgs, LogFailureArgs,
    LogSuccessArgs, TracingArgs, View,
};

mod wire;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> HelperResult {
        match &input.bedrock_decisions {
            Command::ShouldRaise(args) => encode(should_raise_blocked(&args.response)),
            Command::BlockedAssessments(args) => encode(blocked_assessments(&args.response)),
            Command::ViolationNames(args) => encode(violation_category_names(&args.response)),
            Command::HasException(args) => encode(has_exception(&args.view)),
            Command::Status(args) => {
                encode(response_status(args.status_code, args.body.as_bytes()))
            }
            Command::TracingDetail(args) => tracing_detail_view(args),
            Command::HttpException(args) => http_exception_view(args),
            Command::LogAttempt(args) => log_attempt_view(args),
            Command::LogSuccess(args) => log_success_view(args),
            Command::LogFailure(args) => log_failure_view(args),
        }
    }
}

struct NoSpanSink;

impl GuardrailSpanSink for NoSpanSink {
    fn emit(&self, _record: &GuardrailInvocationRecord) -> Result<(), GuardrailSpanError> {
        Ok(())
    }
}

fn has_exception(view: &View) -> bool {
    match view {
        View::Json { payload } => payload_has_exception(payload),
        View::Content { body } => body_has_exception(body.as_bytes()),
        View::Text { text } => body_has_exception(text.as_bytes()),
    }
}

fn tracing_detail_view(args: &TracingArgs) -> HelperResult {
    let catalog = BundledCatalog::default().get();
    let pricing = Pricing {
        catalog: &catalog,
        aws_region_name: args.aws_region_name.as_deref(),
    };
    let detail = tracing_detail(&args.response, &pricing)
        .map_err(|error| failure("BedrockDecisionError", error.to_string()))?;
    encode(detail)
}

fn http_exception_view(args: &HttpExceptionArgs) -> HelperResult {
    let guardrail = BlockedGuardrail {
        identifier: args.guardrail.identifier.as_deref(),
        version: args.guardrail.version.as_deref(),
        name: args.guardrail.name.as_deref(),
        disable_exception_on_block: args.guardrail.disable_exception_on_block,
    };
    match http_exception_for_blocked_guardrail(&args.response, &args.request_data, &guardrail) {
        GuardrailFailure::HttpObject { status, detail } => {
            encode(FailureView::Http { status, detail })
        }
        GuardrailFailure::ModifyResponse(failure) => encode(FailureView::Modify {
            message: failure.message.clone(),
            model: failure.model.clone(),
            request_data: failure.request_data.clone(),
            guardrail_name: failure.guardrail_name.clone(),
            detection_info: failure.detection_info.clone(),
        }),
        GuardrailFailure::GuardrailRaised { .. }
        | GuardrailFailure::BlockedPii { .. }
        | GuardrailFailure::SensitiveDataRoute(_)
        | GuardrailFailure::Http { .. }
        | GuardrailFailure::Other(_) => Err(unexpected_failure()),
    }
}

fn log_attempt_view(args: &LogAttemptArgs) -> HelperResult {
    let catalog = BundledCatalog::default().get();
    let call = call_log(&catalog);
    let base = guardrail_base(&args.guardrail);
    let sink = NoSpanSink;
    let mut request_data = args.request_data.clone();
    let mut context = GuardrailRecordContext {
        base: &base,
        request_data: &mut request_data,
        log_target: None,
        sink: &sink,
    };
    let attempt = AttemptLog {
        status_code: args.status_code,
        body: args.body.as_bytes(),
        json_response: &args.response,
        completed_chunk_usages: &args.completed_chunk_usages,
    };
    log_attempt(&mut context, &call, &attempt)
        .map_err(|error| failure("BedrockDecisionError", error.to_string()))?;
    encode(request_data)
}

fn log_success_view(args: &LogSuccessArgs) -> HelperResult {
    let catalog = BundledCatalog::default().get();
    let call = call_log(&catalog);
    let base = guardrail_base(&args.guardrail);
    let sink = NoSpanSink;
    let mut request_data = args.request_data.clone();
    let mut context = GuardrailRecordContext {
        base: &base,
        request_data: &mut request_data,
        log_target: None,
        sink: &sink,
    };
    log_success(&mut context, &call, &args.response)
        .map_err(|error| failure("BedrockDecisionError", error.to_string()))?;
    encode(request_data)
}

fn log_failure_view(args: &LogFailureArgs) -> HelperResult {
    let catalog = BundledCatalog::default().get();
    let call = call_log(&catalog);
    let base = guardrail_base(&args.guardrail);
    let sink = NoSpanSink;
    let mut request_data = args.request_data.clone();
    let mut context = GuardrailRecordContext {
        base: &base,
        request_data: &mut request_data,
        log_target: None,
        sink: &sink,
    };
    log_failure(
        &mut context,
        &call,
        &args.detail,
        &args.completed_chunk_usages,
    )
    .map_err(|error| failure("BedrockDecisionError", error.to_string()))?;
    encode(request_data)
}

fn call_log(catalog: &ModelCatalog) -> CallLog<'_> {
    CallLog {
        guardrail_provider: "bedrock",
        event_type: Some(GuardrailEventHooks::DuringCall),
        start_time: SystemTime::UNIX_EPOCH + Duration::from_millis(100_250),
        now: SystemTime::UNIX_EPOCH + Duration::from_millis(102_750),
        pricing: Pricing {
            catalog,
            aws_region_name: None,
        },
    }
}

fn guardrail_base(guardrail: &GuardrailSetup) -> GuardrailBase {
    let settings = GuardrailSettings {
        guardrail_name: guardrail.name.clone(),
        ..GuardrailSettings::default()
    };
    GuardrailBase {
        settings,
        mode: None,
    }
}

fn encode<T>(value: T) -> HelperResult
where
    T: Serialize,
{
    to_value(value).map_err(|error| failure("SerializationError", error.to_string()))
}

fn unexpected_failure() -> Box<HelperError> {
    failure(
        "UnexpectedFailure",
        "the blocked guardrail returns only an HTTP object or a modify response".to_owned(),
    )
}

fn failure(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
