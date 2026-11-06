use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde::Deserialize;

use crate::panw_prisma_airs::metadata::{
    RequestData as MetadataRequest, ScanMetadata, UserMetadata, prepare_metadata_from_request,
};

// The LiteLLM request_data dict this guardrail reads, plus the keys it writes back.
#[derive(Debug, Clone, Default)]
pub struct RequestData {
    pub litellm_call_id: Option<String>,
    pub model: Option<String>,
    pub user: Option<String>,
    pub litellm_trace_id: Option<String>,
    pub metadata: Option<BTreeMap<String, GuardrailValue>>,
    pub mcp_tool_name: Option<String>,
    pub name: Option<String>,
    pub arguments: Option<GuardrailValue>,
    pub mcp_arguments: Option<GuardrailValue>,
    pub mcp_server_name: Option<String>,
    pub server_name: Option<String>,
    pub server_id: Option<String>,
    pub proxy_server_request: Option<ProxyServerRequest>,
    pub instructions: Option<String>,
    pub input: Option<ResponsesInput>,
}

#[derive(Debug, Clone, Default)]
pub struct ProxyServerRequest {
    pub url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ResponsesInput {
    Text(String),
    Items(Vec<ResponsesInputItem>),
}

#[derive(Debug, Clone, Deserialize)]
pub struct ResponsesInputItem {
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub role: Option<String>,
    pub content: Option<ResponsesContent>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ResponsesContent {
    Text(String),
    Parts(Vec<ResponsesPart>),
}

#[derive(Debug, Clone, Deserialize)]
pub struct ResponsesPart {
    pub text: Option<String>,
}

// The LiteLLMLoggingObj fields the unified method reads.
#[derive(Debug, Clone, Default)]
pub struct LoggingObject {
    pub litellm_call_id: Option<String>,
    pub model: Option<String>,
    pub call_type: Option<String>,
    pub model_call_details: Option<ModelCallDetails>,
}

#[derive(Debug, Clone, Default)]
pub struct ModelCallDetails {
    pub litellm_params: Option<LitellmParams>,
}

#[derive(Debug, Clone, Default)]
pub struct LitellmParams {
    pub metadata: Option<BTreeMap<String, GuardrailValue>>,
}

impl RequestData {
    #[must_use]
    pub fn resolved_tool_name(&self) -> Option<&str> {
        self.mcp_tool_name
            .as_deref()
            .filter(|name| !name.is_empty())
            .or_else(|| self.mcp_name_fallback())
    }

    // A bare "name" key without an argument field is a stray chat body field, not an MCP request.
    fn mcp_name_fallback(&self) -> Option<&str> {
        if self.arguments.is_none() && self.mcp_arguments.is_none() {
            return None;
        }
        self.name.as_deref().filter(|name| !name.is_empty())
    }

    #[must_use]
    pub fn scan_metadata(&self) -> ScanMetadata {
        let metadata = self.metadata.as_ref().map(user_metadata);
        prepare_metadata_from_request(&MetadataRequest {
            user: self.user.clone(),
            model: self.model.clone(),
            litellm_trace_id: self.litellm_trace_id.clone(),
            metadata,
        })
    }

    pub fn merge_logging_metadata(&mut self, logging_obj: Option<&LoggingObject>) {
        let Some(base) = logging_obj
            .and_then(logging_metadata)
            .filter(|base| !base.is_empty())
        else {
            return;
        };
        let mut merged = base.clone();
        merged.extend(self.metadata.clone().unwrap_or_default());
        self.metadata = Some(merged);
    }

    pub fn fill_model(&mut self, inputs_model: Option<&str>, logging_obj: Option<&LoggingObject>) {
        if self.model.as_deref().is_some_and(|model| !model.is_empty()) {
            return;
        }
        if let Some(model) = inputs_model.filter(|model| !model.is_empty()) {
            self.model = Some(model.to_owned());
            return;
        }
        if let Some(logging) = logging_obj {
            self.model.clone_from(&logging.model);
        }
    }
}

fn logging_metadata(logging_obj: &LoggingObject) -> Option<&BTreeMap<String, GuardrailValue>> {
    logging_obj
        .model_call_details
        .as_ref()?
        .litellm_params
        .as_ref()?
        .metadata
        .as_ref()
}

fn user_metadata(entries: &BTreeMap<String, GuardrailValue>) -> UserMetadata {
    UserMetadata {
        profile_name: text_field(entries, "profile_name"),
        profile_id: text_field(entries, "profile_id"),
        user_ip: text_field(entries, "user_ip"),
        app_name: text_field(entries, "app_name"),
        app_user: text_field(entries, "app_user"),
        litellm_trace_id: text_field(entries, "litellm_trace_id"),
        trace_id: text_field(entries, "trace_id"),
        requester_metadata: nested_object(entries, "requester_metadata")
            .map(|nested| Box::new(user_metadata(nested))),
    }
}

fn text_field(entries: &BTreeMap<String, GuardrailValue>, key: &str) -> Option<String> {
    match entries.get(key) {
        Some(GuardrailValue::Text(text)) => Some(text.clone()),
        _ => None,
    }
}

fn nested_object<'entries>(
    entries: &'entries BTreeMap<String, GuardrailValue>,
    key: &str,
) -> Option<&'entries BTreeMap<String, GuardrailValue>> {
    match entries.get(key) {
        Some(GuardrailValue::Object(nested)) => Some(nested),
        _ => None,
    }
}
