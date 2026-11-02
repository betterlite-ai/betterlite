use serde_json::Value;

use super::cache_operations::SandboxCache;
use super::lifecycle::{SandboxConfig, SandboxProvider, SandboxRegistry};
use super::resolution::SandboxToolRegistry;
use crate::code_execution_records::responses_dialect;

#[derive(Debug, Clone, Default)]
pub struct PlanMetadata {
    pub is_session_scoped: bool,
    pub sandbox_key: Option<String>,
    pub code_interpreter_calls: Vec<Value>,
}

fn merge_into_response(mut response: Value, calls: &[Value]) -> Value {
    let Some(object) = response.as_object_mut() else {
        return response;
    };
    let Some(output) = object.get("output") else {
        return response;
    };
    if let Some(merged) = responses_dialect::merge_calls(output, calls) {
        object.insert("output".to_owned(), Value::Array(merged));
    }
    response
}

pub async fn cleanup_hook<C, R, T>(
    provider: &SandboxProvider<'_, C, R, T>,
    cache: &mut SandboxCache,
    metadata: &PlanMetadata,
) where
    C: SandboxConfig,
    R: SandboxRegistry<Error = C::Error>,
    T: SandboxToolRegistry,
{
    if metadata.is_session_scoped {
        return;
    }
    provider
        .delete_container_for_cache_key(cache, metadata.sandbox_key.as_deref())
        .await;
}

pub async fn post_response_hook<C, R, T>(
    provider: &SandboxProvider<'_, C, R, T>,
    cache: &mut SandboxCache,
    metadata: &PlanMetadata,
    response: Value,
) -> Value
where
    C: SandboxConfig,
    R: SandboxRegistry<Error = C::Error>,
    T: SandboxToolRegistry,
{
    if !metadata.is_session_scoped {
        provider
            .delete_container_for_cache_key(cache, metadata.sandbox_key.as_deref())
            .await;
    }
    if metadata.code_interpreter_calls.is_empty() {
        return response;
    }
    merge_into_response(response, &metadata.code_interpreter_calls)
}

mod tests;
