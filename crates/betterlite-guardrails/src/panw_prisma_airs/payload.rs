use betterlite_config::sections::panw_prisma_airs::Configured;
use serde::Serialize;
use serde_json::value::RawValue;

use super::audit::Direction;
use super::metadata::ScanMetadata;

const DEFAULT_USER: &str = "litellm_user";
const DEFAULT_MODEL: &str = "unknown";

// One scan: the text (or the MCP tool_event that replaces it) and who it is for.
#[derive(Debug, Clone, Copy)]
pub struct ScanRequest<'req> {
    pub content: &'req str,
    pub direction: Direction,
    pub metadata: Option<&'req ScanMetadata>,
    pub call_id: Option<&'req str>,
    pub tool_event: Option<&'req RawValue>,
}

#[derive(Debug, Serialize)]
pub struct ScanPayload<'req> {
    metadata: PayloadMetadata<'req>,
    contents: [Content<'req>; 1],
    #[serde(skip_serializing_if = "Option::is_none")]
    tr_id: Option<&'req str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ai_profile: Option<AiProfile<'req>>,
}

#[derive(Debug, Serialize)]
struct PayloadMetadata<'req> {
    app_user: &'req str,
    ai_model: &'req str,
    app_name: String,
    source: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_ip: Option<&'req str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    litellm_trace_id: Option<&'req str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    is_response: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum Content<'req> {
    Prompt(&'req str),
    Response(&'req str),
    ToolEvent(&'req RawValue),
}

#[derive(Debug, Serialize)]
struct AiProfile<'req> {
    #[serde(skip_serializing_if = "Option::is_none")]
    profile_id: Option<&'req str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    profile_name: Option<&'req str>,
}

// The body half of LiteLLM's fn _call_panw_api.
#[must_use]
pub fn build_payload<'req>(
    settings: &'req Configured,
    request: &ScanRequest<'req>,
) -> ScanPayload<'req> {
    let contents = match (request.tool_event, request.direction) {
        (Some(tool_event), _) => Content::ToolEvent(tool_event),
        (None, Direction::Response) => Content::Response(request.content),
        (None, Direction::Request) => Content::Prompt(request.content),
    };
    // Per-request profile_id wins, then per-request profile_name, then the configured profile_name.
    let profile_id = request
        .metadata
        .and_then(|found| found.profile_id.as_deref());
    let profile_name = request
        .metadata
        .and_then(|found| found.profile_name.as_deref())
        .or(settings.profile_name.as_deref());
    let ai_profile = (profile_id.is_some() || profile_name.is_some()).then_some(AiProfile {
        profile_id,
        profile_name,
    });
    ScanPayload {
        metadata: payload_metadata(settings, request),
        contents: [contents],
        tr_id: request.call_id,
        ai_profile,
    }
}

fn payload_metadata<'req>(
    settings: &Configured,
    request: &ScanRequest<'req>,
) -> PayloadMetadata<'req> {
    let metadata = request.metadata;
    let app_name = metadata
        .and_then(|found| found.app_name.as_deref())
        .map_or_else(
            || settings.app_name.clone(),
            |name| format!("LiteLLM-{name}"),
        );
    PayloadMetadata {
        app_user: metadata.map_or(DEFAULT_USER, |found| {
            found.app_user.as_deref().unwrap_or(&found.user)
        }),
        ai_model: metadata.map_or(DEFAULT_MODEL, |found| &found.model),
        app_name,
        source: "litellm_builtin_guardrail",
        user_ip: metadata.and_then(|found| {
            found
                .user_ip
                .as_deref()
                .or(found.requester_ip_address.as_deref())
        }),
        litellm_trace_id: metadata.and_then(|found| found.litellm_trace_id.as_deref()),
        is_response: (request.direction == Direction::Response && request.tool_event.is_none())
            .then_some(true),
    }
}
