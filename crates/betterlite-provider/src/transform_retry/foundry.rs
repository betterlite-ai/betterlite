use std::str;

use betterlite_foundry::rejected_fields::{FoundryRepair, RepairError, repair};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::controls::call_params::normalize_drop_params;
use betterlite_openai::openai_base_config::Object;

use super::TransformOnUnprocessable;
use crate::foundry_dispatch::AzureAi;
use crate::operation::OperationError;
use crate::prepared::PreparedRequest;
use crate::prepared::deployment::Deployment;

impl TransformOnUnprocessable for AzureAi {
    // LiteLLM asks this policy for every HTTP error status; the markers in the body alone decide
    fn should_retry(&self, response: &UpstreamResponse, deployment: &Deployment) -> bool {
        foundry_repair(deployment).should_retry(error_text(response))
    }

    fn transform_request(
        &self,
        prepared: &mut PreparedRequest,
        response: &UpstreamResponse,
        deployment: &Deployment,
    ) -> Result<(), OperationError> {
        // LiteLLM calls this transform only from the chat completion handler, so an embedding body stays as prepared
        let (PreparedRequest::Chat(chat) | PreparedRequest::Stream(chat)) = prepared else {
            return Ok(());
        };
        let sent: Object = serde_json::from_slice(&chat.http.body)
            .map_err(|source| OperationError::Encode { source })?;
        let repaired = repair(error_text(response), &sent).map_err(repair_error)?;
        // only the body changes: the URL, the auth headers and the deadline stay as prepared
        chat.http.body =
            serde_json::to_vec(&repaired).map_err(|source| OperationError::Encode { source })?;
        Ok(())
    }
}

fn foundry_repair(deployment: &Deployment) -> FoundryRepair {
    FoundryRepair {
        call: normalize_drop_params(deployment.drop_params.as_ref()).unwrap_or(false),
        global: deployment.global_drop_params,
    }
}

fn error_text(response: &UpstreamResponse) -> &str {
    str::from_utf8(&response.body).unwrap_or_default()
}

fn repair_error(error: RepairError) -> OperationError {
    match error {
        RepairError::Rejected(source) => OperationError::RejectedFields { source },
        RepairError::Messages { source } => OperationError::Encode { source },
    }
}
