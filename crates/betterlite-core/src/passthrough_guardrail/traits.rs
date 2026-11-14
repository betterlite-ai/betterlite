use std::error::Error as StdError;
use std::future::Future;

use betterlite_models::decisions::Json;

use super::settings::{
    PassThroughGuardrailSettings, PassthroughGuardrailInputs, PassthroughInputType, RequestData,
};

// The logging object LiteLLM stores passthrough_guardrails_config on
pub trait PassthroughLogging {
    fn passthrough_guardrails_config(&self) -> Option<&Json>;
}

// B3601's passthrough_policy operations; betterlite-guardrails depends on this crate, so they arrive here as a port
pub trait PassthroughPolicy: Sync {
    fn get_settings(
        &self,
        config: &Json,
        guardrail_name: &str,
    ) -> Option<PassThroughGuardrailSettings>;

    fn extract_fields(&self, data: &Json, expressions: &[String]) -> String;

    // B3661's transform_user_api_key_dict_to_metadata, which betterlite-guardrails owns for the same reason
    fn user_key_metadata(&self, user_api_key_dict: Option<&RequestData>) -> RequestData;
}

// LiteLLM's CustomGuardrail.apply_guardrail, reduced to the check the passthrough handler runs
pub trait PassthroughGuardrail: Sync {
    type Failure: StdError + Send + Sync + 'static;
    type LogTarget: PassthroughLogging + Send;

    fn guardrail_name(&self) -> Option<&str>;

    fn apply_guardrail(
        &self,
        inputs: PassthroughGuardrailInputs,
        request_data: &mut RequestData,
        input_type: PassthroughInputType,
        logging_obj: Option<&mut Self::LogTarget>,
    ) -> impl Future<Output = Result<PassthroughGuardrailInputs, Self::Failure>> + Send;
}
