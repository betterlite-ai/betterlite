use betterlite_models::catalog::ModelCatalog;
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::code_execution_records::metadata_filtering::{strip, write};
use crate::code_execution_records::tool_shapes::{function_tool_choice, tool_choice_detection};
use crate::code_execution_records::{
    CONVERTED_STREAM_KEY, CallDialect, INTERCEPTION_ACTIVE_KEY, SANDBOX_KEY, SESSION_SCOPED_KEY,
    followup_filtering, helpers,
};

use super::initialization::InterceptionLogger;
use super::provider::resolve_provider;

// The call types LiteLLM's pre-call hook accepts
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallType {
    Completion,
    Acompletion,
    Responses,
    Aresponses,
}

impl CallType {
    const fn dialect(self) -> CallDialect {
        match self {
            Self::Completion | Self::Acompletion => CallDialect::ChatCompletion,
            Self::Responses | Self::Aresponses => CallDialect::Responses,
        }
    }
}

pub fn pre_call_deployment_hook<C>(
    logger: &InterceptionLogger<C>,
    kwargs: &mut Map<String, Value>,
    call_type: Option<CallType>,
) -> Result<bool, serde_json::Error> {
    strip_root_metadata(kwargs);
    let Some(dialect) = interception_dialect(logger, kwargs, call_type) else {
        return Ok(false);
    };
    seed_sandbox_metadata(kwargs);
    rewrite_tools(kwargs, dialect)?;
    Ok(true)
}

fn interception_dialect<C>(
    logger: &InterceptionLogger<C>,
    kwargs: &Map<String, Value>,
    call_type: Option<CallType>,
) -> Option<CallDialect> {
    let requested = call_type?;
    if !logger.enabled {
        return None;
    }
    if !provider_allowed(&logger.catalog, logger.enabled_providers.as_deref(), kwargs) {
        return None;
    }
    if !has_code_interpreter_tool(kwargs) {
        return None;
    }
    Some(requested.dialect())
}

fn provider_allowed(
    catalog: &ModelCatalog,
    enabled_providers: Option<&[String]>,
    kwargs: &Map<String, Value>,
) -> bool {
    let Some(providers) = enabled_providers else {
        return true;
    };
    resolve_provider(catalog, kwargs).is_some_and(|provider| {
        providers
            .iter()
            .any(|enabled| enabled.as_str() == provider.as_str())
    })
}

fn has_code_interpreter_tool(kwargs: &Map<String, Value>) -> bool {
    let Some(tools) = kwargs.get("tools").and_then(Value::as_array) else {
        return false;
    };
    tools.iter().any(is_code_interpreter_tool)
}

fn is_code_interpreter_tool(tool: &Value) -> bool {
    tool.as_object()
        .and_then(|object| object.get("type"))
        .and_then(Value::as_str)
        == Some("code_interpreter")
}

fn strip_root_metadata(kwargs: &mut Map<String, Value>) {
    if !at_root_depth(kwargs.get("_agentic_loop_depth")) {
        return;
    }
    kwargs.remove(INTERCEPTION_ACTIVE_KEY);
    kwargs.remove(SANDBOX_KEY);
    strip(kwargs);
}

fn at_root_depth(depth: Option<&Value>) -> bool {
    match depth {
        None | Some(Value::Null) => true,
        Some(Value::Bool(set)) => !*set,
        Some(Value::Number(number)) => number.as_u64() == Some(0),
        Some(Value::String(text)) => text.is_empty(),
        Some(Value::Array(items)) => items.is_empty(),
        Some(Value::Object(entries)) => entries.is_empty(),
    }
}

fn extract_session_id(kwargs: &Map<String, Value>) -> Option<String> {
    ["metadata", "litellm_metadata"]
        .into_iter()
        .find_map(|key| {
            kwargs
                .get(key)
                .and_then(|metadata| metadata.get("session_id"))
                .and_then(Value::as_str)
                .filter(|session_id| !session_id.is_empty())
                .map(str::to_owned)
        })
}

fn seed_sandbox_metadata(kwargs: &mut Map<String, Value>) {
    kwargs.insert(INTERCEPTION_ACTIVE_KEY.to_owned(), Value::Bool(true));
    if let Some(session_id) = extract_session_id(kwargs) {
        let identity = helpers::extract_identity(kwargs);
        let sandbox_key = if identity.is_empty() {
            session_id
        } else {
            format!("{identity}:{session_id}")
        };
        kwargs.insert(SANDBOX_KEY.to_owned(), Value::String(sandbox_key));
        kwargs.insert(SESSION_SCOPED_KEY.to_owned(), Value::Bool(true));
    } else {
        let sandbox_key = Uuid::new_v4().simple().to_string();
        kwargs.insert(SANDBOX_KEY.to_owned(), Value::String(sandbox_key));
    }
    if matches!(kwargs.get("stream"), Some(Value::Bool(true))) {
        kwargs.insert("stream".to_owned(), Value::Bool(false));
        kwargs.insert(CONVERTED_STREAM_KEY.to_owned(), Value::Bool(true));
    }
    write(kwargs);
}

fn rewrite_tools(
    kwargs: &mut Map<String, Value>,
    dialect: CallDialect,
) -> Result<(), serde_json::Error> {
    let rewritten = kwargs
        .get("tools")
        .and_then(|tools| followup_filtering::tools(tools, dialect));
    if let Some(replaced) = rewritten {
        kwargs.insert("tools".to_owned(), Value::Array(replaced));
    }
    let targets_code_interpreter = kwargs.get("tool_choice").is_some_and(tool_choice_detection);
    if targets_code_interpreter {
        let serialized = serde_json::to_value(function_tool_choice(dialect))?;
        kwargs.insert("tool_choice".to_owned(), serialized);
    }
    Ok(())
}

include!("pre_call/tests/cases.rs");
