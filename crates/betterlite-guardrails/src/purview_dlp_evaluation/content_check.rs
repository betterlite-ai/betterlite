use betterlite_prompts::arguments::json::{Json, Object};

use crate::purview_dlp_evaluation::MicrosoftPurviewDLPGuardrail;
use crate::purview_dlp_evaluation::error_handling::{self, PurviewDlpHttpError};
use crate::purview_graph_client::error::PurviewGraphError;
use crate::purview_graph_client::policy_action::should_block;
use crate::purview_graph_client::process_content::{ContentRequest, process_content};
use crate::purview_graph_client::scope_cache::compute_protection_scopes;
use crate::purview_graph_client::wire::{Activity, ProcessContentResponse};

pub struct ContentCheck<'request> {
    pub user_id: &'request str,
    pub text: &'request str,
    pub activity: Activity,
    pub request_data: &'request Object,
    pub block_on_violation: bool,
}

impl MicrosoftPurviewDLPGuardrail {
    pub async fn check_content(
        &self,
        check: &ContentCheck<'_>,
    ) -> Result<ProcessContentResponse, PurviewDlpHttpError> {
        let (etag, _protection_scopes) =
            match compute_protection_scopes(&self.base, check.user_id).await {
                Ok(scopes) => scopes,
                Err(source) => {
                    return evaluation_failure(&source, check.activity, check.block_on_violation);
                }
            };
        let correlation_id = check
            .request_data
            .get("litellm_call_id")
            .and_then(Json::as_str)
            .filter(|call_id| !call_id.is_empty());
        let request = ContentRequest {
            user_id: check.user_id,
            text: check.text,
            activity: check.activity,
            etag: etag.as_str(),
            correlation_id,
        };
        let response = match process_content(&self.base, &request).await {
            Ok(response) => response,
            Err(source) => {
                return evaluation_failure(&source, check.activity, check.block_on_violation);
            }
        };
        if check.block_on_violation && should_block(&response) {
            return Err(error_handling::policy_block(check.activity));
        }
        Ok(response)
    }
}

fn evaluation_failure(
    error: &PurviewGraphError,
    activity: Activity,
    block_on_violation: bool,
) -> Result<ProcessContentResponse, PurviewDlpHttpError> {
    if block_on_violation {
        return Err(error_handling::evaluation_http_error(error, activity));
    }
    tracing::warn!("Purview DLP: API/network error in logging-only mode (not re-raised): {error}");
    Ok(ProcessContentResponse {
        policy_actions: Vec::new(),
        protection_scope_state: None,
    })
}

mod tests;
