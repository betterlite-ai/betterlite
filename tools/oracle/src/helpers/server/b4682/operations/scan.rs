use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_server::mcp::catalog_guard::guard::{
    CallerContext, GuardrailCallContext, GuardrailFailure, GuardrailHook, ScanContext,
};
use betterlite_server::mcp::catalog_guard::scan_tool_descriptions;
use serde_json::{Value, json};

use super::super::input::{GuardedInput, HookOutcome, ServerInput, ToolInput};
use super::{convert_tools, error, to_server};

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

pub(in crate::helpers::server::b4682) fn run_scan(
    inputs: &[ToolInput],
    outcomes: &BTreeMap<String, HookOutcome>,
    credentials: &[String],
    headers: &BTreeMap<String, String>,
    batch_report: bool,
) -> crate::helpers::HelperResult {
    let tools = convert_tools(inputs)?;
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
