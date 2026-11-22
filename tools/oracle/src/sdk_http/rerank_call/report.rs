use super::super::error::SdkHttpError;
use super::{Failure, RunOutcome};
use betterlite_core::api::rerank_provider_plan::plan::RerankException;
use betterlite_error::envelope::WireField;
use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::RerankResponse;
use betterlite_sdk::call_wrapper::error::class_name;

pub(super) fn outcome(
    result: Result<RerankResponse, RerankException>,
) -> Result<RunOutcome, SdkHttpError> {
    match result {
        Ok(response) => {
            let record = serde_json::to_value(&response).map_err(SdkHttpError::Serialize)?;
            let value: Json = serde_json::from_value(record).map_err(SdkHttpError::Serialize)?;
            Ok(RunOutcome {
                result: Some(value),
                failure: None,
            })
        }
        Err(exception) => Ok(RunOutcome {
            result: None,
            failure: Some(failure(&exception)),
        }),
    }
}

// The exception the SDK raises for this class, as the pinned litellm.exceptions module names it
fn failure(exception: &RerankException) -> Failure {
    let metadata = exception.class.metadata();
    Failure {
        class: class_name(&exception.class).to_owned(),
        status: Some(metadata.status_code),
        kind: text_field(metadata.error_type),
        param: text_field(metadata.param),
        code: metadata.openai_code,
        message: exception.message.clone(),
    }
}

fn text_field(field: WireField) -> Option<String> {
    match field {
        WireField::Text(text) => Some(text),
        WireField::Missing(_) | WireField::Null => None,
    }
}
