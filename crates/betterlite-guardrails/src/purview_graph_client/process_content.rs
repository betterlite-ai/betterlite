use std::sync::PoisonError;

use betterlite_http::url_path::encode_segment;
use betterlite_prompts::arguments::json::Object;
use reqwest::header::{HeaderMap, HeaderValue, IF_NONE_MATCH};

use crate::purview_graph_client::PurviewGuardrailBase;
use crate::purview_graph_client::error::PurviewGraphError;
use crate::purview_graph_client::graph_api::graph_post;
use crate::purview_graph_client::wire::{
    Activity, ActivityMetadata, ContentEntry, ContentToProcess, IntegratedAppMetadata,
    PolicyLocation, ProcessContentRequest, ProcessContentResponse, ProtectedAppMetadata,
    ProtectionScopeState, TextContent,
};

pub struct ContentRequest<'req> {
    pub user_id: &'req str,
    pub text: &'req str,
    pub activity: Activity,
    pub etag: &'req str,
    pub correlation_id: Option<&'req str>,
}

fn content_body<'req>(
    base: &'req PurviewGuardrailBase,
    request: &'req ContentRequest<'req>,
) -> ProcessContentRequest<'req> {
    let application_location = PolicyLocation {
        odata_type: "microsoft.graph.policyLocationApplication",
        value: &base.client_id,
    };
    ProcessContentRequest {
        content_to_process: ContentToProcess {
            content_entries: [ContentEntry {
                odata_type: "microsoft.graph.processConversationMetadata",
                identifier: (base.new_id)().to_string(),
                content: TextContent {
                    odata_type: "microsoft.graph.textContent",
                    data: request.text,
                },
                name: format!("{} message", base.purview_app_name),
                correlation_id: request
                    .correlation_id
                    .map_or_else(|| (base.new_id)().to_string(), str::to_owned),
                sequence_number: 0,
                is_truncated: false,
            }],
            activity_metadata: ActivityMetadata {
                activity: request.activity,
            },
            device_metadata: Object::new(),
            protected_app_metadata: ProtectedAppMetadata {
                name: &base.purview_app_name,
                version: "1.0",
                application_location,
            },
            integrated_app_metadata: IntegratedAppMetadata {
                name: &base.purview_app_name,
                version: "1.0",
            },
        },
    }
}

pub async fn process_content(
    base: &PurviewGuardrailBase,
    request: &ContentRequest<'_>,
) -> Result<ProcessContentResponse, PurviewGraphError> {
    let encoded_user_id = encode_segment(request.user_id, "user_id")
        .map_err(|source| PurviewGraphError::UserId { source })?;
    let url = format!(
        "{}/users/{encoded_user_id}/dataSecurityAndGovernance/processContent",
        base.graph_api_base
    );
    let body = content_body(base, request);

    let mut extra_headers = HeaderMap::new();
    if !request.etag.is_empty() {
        let etag =
            HeaderValue::from_str(request.etag).map_err(|source| PurviewGraphError::Header {
                name: "If-None-Match",
                source,
            })?;
        extra_headers.insert(IF_NONE_MATCH, etag);
    }

    let (answer, _): (ProcessContentResponse, HeaderMap) =
        graph_post(base, &url, &body, &extra_headers).await?;

    // A changed policy makes the cached scopes stale, so the next call fetches them again.
    if answer.protection_scope_state == Some(ProtectionScopeState::Modified) {
        base.scope_and_token_cache_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .protection_scope_cache
            .pop(request.user_id);
    }
    Ok(answer)
}

mod tests;
