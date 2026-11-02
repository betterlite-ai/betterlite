use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_db::mcp_stored_contracts::{JsonValue, PinnedMCPTool};
use betterlite_server::mcp::catalog_guard::guard::{
    CallerContext, GuardrailCallContext, GuardrailFailure, GuardrailHook, ScanContext,
};
use betterlite_server::mcp::catalog_guard::headers::{
    HeaderSanitizeContext, logging_safe_mcp_headers,
};
use betterlite_server::mcp::catalog_guard::pin::pin_tool_catalog;
use betterlite_server::mcp::catalog_guard::prefix::{PrefixPolicy, strip_known_server_prefix};
use betterlite_server::mcp::catalog_guard::types::{MCPServer, MCPTool};
use betterlite_server::mcp::catalog_guard::{block_reason, scan_tool_descriptions};
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    StripPrefix {
        name: String,
        #[serde(default)]
        server: Option<ServerInput>,
    },
    SanitizeHeaders {
        #[serde(default)]
        headers: BTreeMap<String, String>,
        #[serde(default)]
        upstream_credential_names: Vec<String>,
        #[serde(default)]
        identity_header_names: Vec<String>,
        #[serde(default = "default_auth_header")]
        client_side_auth_header_name: String,
        #[serde(default)]
        custom_key_header_name: Option<String>,
    },
    Pin {
        tools: Vec<ToolInput>,
        pinned: BTreeMap<String, PinnedInput>,
    },
    Override {
        tools: Vec<ToolInput>,
        overrides: BTreeMap<String, String>,
        #[serde(default)]
        prefix: Option<String>,
    },
    #[serde(alias = "batch")]
    Scan {
        tools: Vec<ToolInput>,
        #[serde(default)]
        outcomes: BTreeMap<String, HookOutcome>,
        #[serde(default)]
        upstream_credential_names: Vec<String>,
        #[serde(default)]
        headers: BTreeMap<String, String>,
        #[serde(default)]
        batch_report: bool,
    },
    Error {
        failure: GuardrailFailure,
    },
}

#[derive(Deserialize)]
pub(crate) struct ToolInput {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default, alias = "input_schema")]
    input_schema: BTreeMap<String, Value>,
    #[serde(default, alias = "output_schema")]
    output_schema: Option<BTreeMap<String, Value>>,
    #[serde(default)]
    execution: Option<BTreeMap<String, Value>>,
    #[serde(default)]
    annotations: Option<BTreeMap<String, Value>>,
    #[serde(default)]
    icons: Option<Vec<Value>>,
    #[serde(default, rename = "_meta")]
    meta: Option<BTreeMap<String, Value>>,
}

#[derive(Deserialize)]
pub(crate) struct PinnedInput {
    #[serde(default)]
    description: String,
    #[serde(default, alias = "input_schema")]
    input_schema: BTreeMap<String, Value>,
}

#[derive(Deserialize, Default)]
pub(crate) struct ServerInput {
    #[serde(default)]
    server_id: String,
    #[serde(default = "default_server_name")]
    name: String,
    #[serde(default)]
    alias: Option<String>,
    #[serde(default)]
    server_name: Option<String>,
    #[serde(default)]
    short_prefix: Option<String>,
}

#[derive(Clone, Deserialize, Default)]
pub(crate) struct HookOutcome {
    #[serde(default)]
    guarded: Option<GuardedInput>,
    #[serde(default)]
    failure: Option<GuardrailFailure>,
}

#[derive(Clone, Deserialize, Default)]
pub(crate) struct GuardedInput {
    #[serde(default, rename = "mcp_tool_description")]
    description: Option<Value>,
    #[serde(default, rename = "mcp_tool_description_present")]
    description_present: bool,
    #[serde(default, rename = "mcp_input_schema")]
    schema: Option<Value>,
    #[serde(default, rename = "mcp_input_schema_present")]
    schema_present: bool,
}

#[derive(Default)]
struct ControlledHook {
    outcomes: BTreeMap<String, HookOutcome>,
    active: Arc<AtomicUsize>,
    maximum: Arc<AtomicUsize>,
}

impl GuardrailHook for ControlledHook {
    fn pre_call_hook(
        &self,
        context: GuardrailCallContext<'_>,
    ) -> impl Future<Output = Result<Option<BTreeMap<String, JsonValue>>, GuardrailFailure>> + Send
    {
        let outcome = self
            .outcomes
            .get(context.tool_name)
            .cloned()
            .unwrap_or_default();
        let active = Arc::clone(&self.active);
        let maximum = Arc::clone(&self.maximum);
        let guarded = outcome.guarded.map(to_guarded_map).transpose();
        async move {
            let previous = active.fetch_add(1, Ordering::SeqCst);
            let current = previous.saturating_add(1);
            maximum.fetch_max(current, Ordering::SeqCst);
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
            active.fetch_sub(1, Ordering::SeqCst);
            outcome.failure.map_or(guarded, Err)
        }
    }
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::StripPrefix { name, server } => Ok(run_prefix(name, server.as_ref())),
            Input::SanitizeHeaders {
                headers,
                upstream_credential_names,
                identity_header_names,
                client_side_auth_header_name,
                custom_key_header_name,
            } => sanitize_headers(
                headers,
                upstream_credential_names,
                identity_header_names,
                client_side_auth_header_name,
                custom_key_header_name.as_deref(),
            ),
            Input::Pin { tools, pinned } => run_pin(tools, pinned),
            Input::Override {
                tools,
                overrides,
                prefix,
            } => run_override(tools, overrides, prefix.as_deref()),
            Input::Scan {
                tools,
                outcomes,
                upstream_credential_names,
                headers,
                batch_report,
            } => run_scan(
                tools,
                outcomes,
                upstream_credential_names,
                headers,
                *batch_report,
            ),
            Input::Error { failure } => Ok(json!({"reason": block_reason(failure)})),
        }
    }
}

fn run_prefix(name: &str, server: Option<&ServerInput>) -> Value {
    let configured_server = server.map(to_server);
    let bare_name =
        strip_known_server_prefix(name, configured_server.as_ref(), &PrefixPolicy::default());
    Value::String(bare_name)
}

fn sanitize_headers(
    headers: &BTreeMap<String, String>,
    credentials: &[String],
    identity: &[String],
    client_auth: &str,
    custom_key: Option<&str>,
) -> crate::helpers::HelperResult {
    let context = HeaderSanitizeContext {
        upstream_credential_names: credentials,
        identity_header_names: identity,
        custom_key_header_name: custom_key,
        client_side_auth_header_name: client_auth,
    };
    serde_json::to_value(logging_safe_mcp_headers(headers, &context))
        .map_err(|source| error("SerializationError", source.to_string()))
}

fn run_pin(
    inputs: &[ToolInput],
    pin_inputs: &BTreeMap<String, PinnedInput>,
) -> crate::helpers::HelperResult {
    let tools: Vec<MCPTool> = inputs
        .iter()
        .map(MCPTool::try_from)
        .collect::<Result<_, _>>()?;
    let pinned = pin_inputs
        .iter()
        .map(|(name, value)| Ok((name.clone(), to_pinned(value)?)))
        .collect::<Result<BTreeMap<_, _>, Box<crate::helpers::HelperError>>>()?;
    let (served, drift) = pin_tool_catalog(&tools, &pinned);
    Ok(json!({
        "served": served,
        "added": drift.as_ref().map_or_else(Vec::new, |value| value.added.clone()),
        "removed": drift.as_ref().map_or_else(Vec::new, |value| value.removed.clone()),
        "changed": drift.as_ref().map_or_else(Vec::new, |value| value.changed.clone()),
    }))
}

fn run_override(
    inputs: &[ToolInput],
    overrides: &BTreeMap<String, String>,
    prefix: Option<&str>,
) -> crate::helpers::HelperResult {
    let tools: Vec<MCPTool> = inputs
        .iter()
        .map(MCPTool::try_from)
        .collect::<Result<_, _>>()?;
    let configured_server = MCPServer {
        alias: prefix.map(str::to_owned),
        tool_name_to_description: overrides.clone(),
        server_id: "server-id".to_owned(),
        name: "server".to_owned(),
        server_name: None,
        short_prefix: None,
        extra_headers: Vec::new(),
    };
    let served =
        betterlite_server::mcp::catalog_guard::description_overrides::apply_description_overrides(
            &tools,
            &configured_server,
            &PrefixPolicy::default(),
        );
    serde_json::to_value(served).map_err(|source| error("SerializationError", source.to_string()))
}

impl TryFrom<&ToolInput> for MCPTool {
    type Error = Box<crate::helpers::HelperError>;

    fn try_from(input: &ToolInput) -> Result<Self, Self::Error> {
        Ok(Self {
            name: input.name.clone(),
            description: input.description.clone(),
            title: input.title.clone(),
            input_schema: json_object(&input.input_schema)?,
            output_schema: input.output_schema.as_ref().map(json_object).transpose()?,
            execution: input.execution.as_ref().map(json_object).transpose()?,
            annotations: input.annotations.as_ref().map(json_object).transpose()?,
            icons: input
                .icons
                .as_ref()
                .map(|values| values.iter().map(json_value).collect())
                .transpose()?,
            meta: input.meta.as_ref().map(json_object).transpose()?,
        })
    }
}

fn to_pinned(input: &PinnedInput) -> Result<PinnedMCPTool, Box<crate::helpers::HelperError>> {
    let input_schema = json_object(&input.input_schema)?;
    let encoded = serde_json::to_string(&json!({
        "description": input.description,
        "input_schema": input_schema,
    }))
    .map_err(|source| error("SerializationError", source.to_string()))?;
    serde_json::from_str(&encoded).map_err(|source| error("InvalidPinnedTool", source.to_string()))
}

fn json_object(
    values: &BTreeMap<String, Value>,
) -> Result<BTreeMap<String, JsonValue>, Box<crate::helpers::HelperError>> {
    values
        .iter()
        .map(|(name, value)| Ok((name.clone(), json_value(value)?)))
        .collect()
}

fn json_value(value: &Value) -> Result<JsonValue, Box<crate::helpers::HelperError>> {
    let encoded = serde_json::to_string(value)
        .map_err(|source| error("SerializationError", source.to_string()))?;
    serde_json::from_str(&encoded).map_err(|source| error("InvalidJsonValue", source.to_string()))
}

fn to_guarded_map(input: GuardedInput) -> Result<BTreeMap<String, JsonValue>, GuardrailFailure> {
    let mut fields = BTreeMap::new();
    insert_guarded_field(
        &mut fields,
        "mcp_tool_description",
        input.description,
        input.description_present,
    )?;
    insert_guarded_field(
        &mut fields,
        "mcp_input_schema",
        input.schema,
        input.schema_present,
    )?;
    Ok(fields)
}

fn insert_guarded_field(
    fields: &mut BTreeMap<String, JsonValue>,
    name: &str,
    value: Option<Value>,
    is_present: bool,
) -> Result<(), GuardrailFailure> {
    match value {
        Some(json_value) => {
            fields.insert(name.to_owned(), json_value_for_guardrail(&json_value)?);
        }
        None if is_present => {
            fields.insert(name.to_owned(), JsonValue::Null);
        }
        None => {}
    }
    Ok(())
}

fn json_value_for_guardrail(value: &Value) -> Result<JsonValue, GuardrailFailure> {
    let encoded = serde_json::to_string(value)
        .map_err(|source| invalid_guarded_response(source.to_string()))?;
    serde_json::from_str(&encoded).map_err(|source| invalid_guarded_response(source.to_string()))
}

fn invalid_guarded_response(message: String) -> GuardrailFailure {
    GuardrailFailure {
        exception_type: "InvalidGuardedResponse".to_owned(),
        exception_message: message,
        detail_error: None,
    }
}

fn run_scan(
    inputs: &[ToolInput],
    outcomes: &BTreeMap<String, HookOutcome>,
    credentials: &[String],
    headers: &BTreeMap<String, String>,
    batch_report: bool,
) -> crate::helpers::HelperResult {
    let tools: Vec<MCPTool> = inputs
        .iter()
        .map(MCPTool::try_from)
        .collect::<Result<_, _>>()?;
    let server = to_server(&ServerInput::default());
    let caller = CallerContext::default();
    let context = ScanContext {
        server: &server,
        caller: Some(&caller),
        headers,
        identity_header_names: &[],
        custom_key_header_name: None,
        client_side_auth_header_name: "x-mcp-auth",
        upstream_credential_names: credentials,
    };
    let hook = ControlledHook {
        outcomes: outcomes.clone(),
        active: Arc::new(AtomicUsize::new(0)),
        maximum: Arc::new(AtomicUsize::new(0)),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|source| error("RuntimeError", source.to_string()))?;
    let result = runtime.block_on(scan_tool_descriptions(&tools, &context, &hook));
    if batch_report {
        let order: Vec<&str> = result
            .served
            .iter()
            .map(|tool| tool.name.as_str())
            .collect();
        Ok(json!({
            "scan": result,
            "batch_size": hook.maximum.load(Ordering::SeqCst),
            "order": order,
        }))
    } else {
        serde_json::to_value(result)
            .map_err(|source| error("SerializationError", source.to_string()))
    }
}

fn to_server(input: &ServerInput) -> MCPServer {
    MCPServer {
        server_id: input.server_id.clone(),
        name: input.name.clone(),
        alias: input.alias.clone(),
        server_name: input.server_name.clone(),
        short_prefix: input.short_prefix.clone(),
        extra_headers: Vec::new(),
        tool_name_to_description: BTreeMap::new(),
    }
}

fn error(class: &str, message: String) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn default_auth_header() -> String {
    "x-mcp-auth".to_owned()
}

fn default_server_name() -> String {
    "server".to_owned()
}

mod tests;
