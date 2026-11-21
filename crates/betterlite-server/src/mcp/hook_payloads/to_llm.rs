use betterlite_core::api::request_destination::RequestData;
use betterlite_core::api::request_guardrail_policy::error::RequestGuardrailPolicyError;
use betterlite_core::api::request_guardrail_policy::{
    GuardrailMetadata, GuardrailPolicyService, GuardrailPolicySettings, GuardrailPrincipal,
    RequestGuardrailPolicyInput,
};
use betterlite_models::decisions::Json;
use betterlite_policy::attachment::AttachmentRegistry;
use betterlite_policy::registry::PolicyRegistry;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Serialize;

use crate::mcp::hook_payloads::error::HookPayloadError;
use crate::mcp::hook_payloads::{McpHookKwargs, McpToolCall, ToolArguments, tool_call_text};

const DEFAULT_MODEL: &str = "mcp-tool-call";
const METADATA: &str = "metadata";
const GUARDRAILS: &str = "guardrails";
const POLICIES: &str = "policies";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    User,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolCallMessage {
    pub role: MessageRole,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct McpGuardrailRequest {
    pub messages: Vec<ToolCallMessage>,
    pub model: String,
    pub user_api_key_user_id: Option<String>,
    pub user_api_key_team_id: Option<String>,
    pub user_api_key_end_user_id: Option<String>,
    pub user_api_key_hash: Option<String>,
    pub user_api_key_request_route: Option<String>,
    pub mcp_tool_name: String,
    pub mcp_arguments: ToolArguments,
    pub mcp_tool_description: Option<String>,
    pub mcp_input_schema: Option<IndexMap<String, Json>>,
    pub mcp_server_name: Option<String>,
    pub incoming_bearer_token: Option<SecretString>,
    pub metadata: IndexMap<String, Json>,
}

pub struct AuthGuardrails<'registry> {
    pub policies: &'registry PolicyRegistry,
    pub settings: GuardrailPolicySettings,
}

pub fn mcp_to_llm_request(
    call: McpToolCall<'_>,
    kwargs: &McpHookKwargs,
    guardrails: &AuthGuardrails<'_>,
) -> Result<McpGuardrailRequest, HookPayloadError> {
    let description = kwargs
        .mcp_tool_description
        .as_deref()
        .filter(|text| !text.is_empty());
    let model = kwargs
        .guardrail_context
        .model
        .clone()
        .or_else(|| kwargs.model.clone())
        .unwrap_or_else(|| DEFAULT_MODEL.to_owned());
    let metadata = request_metadata(kwargs, &model, guardrails)?;
    Ok(McpGuardrailRequest {
        messages: vec![ToolCallMessage {
            role: MessageRole::User,
            content: tool_call_text(call, description)?,
        }],
        model,
        user_api_key_user_id: kwargs.user_api_key_user_id.clone(),
        user_api_key_team_id: kwargs.user_api_key_team_id.clone(),
        user_api_key_end_user_id: kwargs.user_api_key_end_user_id.clone(),
        user_api_key_hash: kwargs.user_api_key_hash.clone(),
        user_api_key_request_route: kwargs.user_api_key_request_route.clone(),
        mcp_tool_name: call.tool_name.to_owned(),
        mcp_arguments: call.arguments.clone(),
        mcp_tool_description: description.map(str::to_owned),
        mcp_input_schema: kwargs.mcp_input_schema.clone(),
        mcp_server_name: kwargs.mcp_rate_limit_server_name.clone(),
        incoming_bearer_token: kwargs.incoming_bearer_token.clone(),
        metadata,
    })
}

fn request_metadata(
    kwargs: &McpHookKwargs,
    model: &str,
    guardrails: &AuthGuardrails<'_>,
) -> Result<IndexMap<String, Json>, HookPayloadError> {
    let parent = &kwargs.guardrail_context.metadata;
    let mut metadata = synthetic_metadata(parent, kwargs);
    if let Some(auth) = &kwargs.user_api_key_auth {
        metadata = with_auth_guardrails(metadata, model, auth, guardrails)?;
    }
    let merged = merged_guardrails(&metadata, parent)?;
    metadata.insert(GUARDRAILS.to_owned(), Json::List(merged));
    Ok(metadata)
}

// the parent request's metadata without its guardrails, which come back merged after the auth ones
fn synthetic_metadata(
    parent: &IndexMap<String, Json>,
    kwargs: &McpHookKwargs,
) -> IndexMap<String, Json> {
    let mut metadata: IndexMap<String, Json> = parent
        .iter()
        .filter(|(key, _)| key.as_str() != GUARDRAILS)
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let headers = kwargs
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
        .collect();
    metadata.insert("headers".to_owned(), Json::Object(headers));
    for (key, value) in [
        ("user_api_key_user_id", &kwargs.user_api_key_user_id),
        ("user_api_key_team_id", &kwargs.user_api_key_team_id),
        ("user_api_key_end_user_id", &kwargs.user_api_key_end_user_id),
    ] {
        metadata.insert(key.to_owned(), value.clone().map_or(Json::Null, Json::Text));
    }
    metadata
}

// add_guardrails_from_auth_metadata is request guardrail selection minus the policy engine, so it runs with no attachments
fn with_auth_guardrails(
    metadata: IndexMap<String, Json>,
    model: &str,
    auth: &IndexMap<String, Json>,
    guardrails: &AuthGuardrails<'_>,
) -> Result<IndexMap<String, Json>, HookPayloadError> {
    let input = auth_policy_input(metadata, model, auth)?;
    let attachments = AttachmentRegistry {
        attachments: Vec::new(),
        config_attachments: Vec::new(),
        initialized: false,
    };
    let service = GuardrailPolicyService {
        policies: guardrails.policies,
        attachments: &attachments,
        settings: guardrails.settings,
    };
    let mut resolved = service.resolve(&input)?.data;
    let Some(Json::Object(mut updated)) = resolved.shift_remove(METADATA) else {
        return Err(HookPayloadError::Guardrails(
            RequestGuardrailPolicyError::MetadataBucket,
        ));
    };
    for (key, field) in [
        ("user_api_key_metadata", "metadata"),
        ("user_api_key_team_metadata", "team_metadata"),
    ] {
        updated.insert(
            key.to_owned(),
            auth.get(field).cloned().unwrap_or(Json::Null),
        );
    }
    Ok(updated)
}

fn auth_policy_input(
    metadata: IndexMap<String, Json>,
    model: &str,
    auth: &IndexMap<String, Json>,
) -> Result<RequestGuardrailPolicyInput, HookPayloadError> {
    let mut data = RequestData::new();
    data.insert("model".to_owned(), Json::Text(model.to_owned()));
    data.insert(METADATA.to_owned(), Json::Object(metadata));
    Ok(RequestGuardrailPolicyInput {
        data,
        principal: GuardrailPrincipal {
            team_alias: None,
            key_alias: None,
        },
        key_metadata: guardrail_metadata(auth, "metadata")?,
        team_metadata: guardrail_metadata(auth, "team_metadata")?,
        project_metadata: guardrail_metadata(auth, "project_metadata")?,
        metadata_variable_name: METADATA.to_owned(),
    })
}

fn guardrail_metadata(
    auth: &IndexMap<String, Json>,
    field: &'static str,
) -> Result<GuardrailMetadata, HookPayloadError> {
    let metadata = match auth.get(field) {
        None | Some(Json::Null) => {
            return Ok(GuardrailMetadata {
                guardrails: Vec::new(),
                policies: Vec::new(),
            });
        }
        Some(Json::Object(found)) => found,
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(HookPayloadError::AuthMetadata {
                field,
                key: GUARDRAILS,
            });
        }
    };
    Ok(GuardrailMetadata {
        guardrails: names(metadata, field, GUARDRAILS)?,
        policies: names(metadata, field, POLICIES)?,
    })
}

fn names(
    metadata: &IndexMap<String, Json>,
    field: &'static str,
    key: &'static str,
) -> Result<Vec<String>, HookPayloadError> {
    let items = match metadata.get(key) {
        None | Some(Json::Null) => return Ok(Vec::new()),
        Some(Json::List(list)) => list,
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            return Err(HookPayloadError::AuthMetadata { field, key });
        }
    };
    items
        .iter()
        .map(|item| match item {
            Json::Text(name) => Ok(name.clone()),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                Err(HookPayloadError::AuthMetadata { field, key })
            }
        })
        .collect()
}

// the auth guardrails first, then the parent request's, each kept once in first-seen order
fn merged_guardrails(
    metadata: &IndexMap<String, Json>,
    parent: &IndexMap<String, Json>,
) -> Result<Vec<Json>, HookPayloadError> {
    let mut merged: Vec<Json> = Vec::new();
    for source in [metadata, parent] {
        let items = match source.get(GUARDRAILS) {
            None | Some(Json::Null) => continue,
            Some(Json::List(list)) => list,
            Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
                return Err(HookPayloadError::GuardrailList);
            }
        };
        for item in items {
            if !merged.contains(item) {
                merged.push(item.clone());
            }
        }
    }
    Ok(merged)
}
