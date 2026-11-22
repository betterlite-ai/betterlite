use betterlite_core::api::speech_call::error::{SpeechCallError, SpeechFailure, exception};
use betterlite_core::api::speech_call_plan::SpeechReturn;
use betterlite_core::api::speech_call_plan::error::AspeechError;
use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;
use betterlite_models::resolution::ResolutionError;
use betterlite_redact::secret::Redactor;

use super::super::error::SdkHttpError;
use super::super::result::{Failure, Nullable};
use super::super::speech_call::{BinaryValue, StepOutcome};
use super::{CasePending, Raised};

// what the SDK caller of aspeech observes for one step
pub(super) fn project(
    result: Result<SpeechReturn<BinaryValue, CasePending<'_>>, AspeechError<Raised>>,
    redactor: &Redactor,
) -> Result<StepOutcome, SdkHttpError> {
    let failure = match result {
        Ok(SpeechReturn::Finished(audio)) => {
            return Ok(StepOutcome {
                value: Some(audio),
                failure: None,
            });
        }
        Ok(SpeechReturn::Coroutine(_)) => {
            return Err(SdkHttpError::Invalid(String::from(
                "aspeech returned a coroutine it never awaited",
            )));
        }
        // str(KeyError("model")) is the quoted key
        Err(AspeechError::MissingModel) => Failure {
            class: String::from("KeyError"),
            status: 0,
            kind: None,
            param: None,
            code: None,
            message: String::from("'model'"),
        },
        Err(AspeechError::Raised {
            error: Raised::Speech(failure),
            ..
        }) => *failure,
        Err(AspeechError::Raised {
            error: Raised::Harness(error),
            ..
        }) => return Err(error),
        Err(AspeechError::Raised {
            model,
            provider,
            error: Raised::Resolution(source),
        }) => resolution_failure(*source, &model, &provider, redactor)?,
    };
    Ok(StepOutcome {
        value: None,
        failure: Some(failure),
    })
}

// get_llm_provider raises litellm.BadRequestError, which exception_type hands back unchanged
fn resolution_failure(
    source: ResolutionError,
    model: &str,
    provider: &str,
    redactor: &Redactor,
) -> Result<Failure, SdkHttpError> {
    let raised = exception(
        &SpeechCallError::Resolution(source),
        model,
        provider,
        redactor,
    );
    let SpeechFailure::Mapped(class @ ErrorClass::BadRequest(_)) = &raised.failure else {
        return Err(SdkHttpError::Unprojected(format!("{:?}", raised.failure)));
    };
    let metadata = class.metadata();
    Ok(Failure {
        class: String::from("BadRequestError"),
        status: metadata.status_code,
        kind: field(metadata.error_type),
        param: field(metadata.param),
        code: Some(metadata.openai_code),
        message: raised.message,
    })
}

fn field(wire: WireField) -> Nullable {
    match wire {
        WireField::Missing(_) => None,
        WireField::Null => Some(None),
        WireField::Text(text) => Some(Some(text)),
    }
}
