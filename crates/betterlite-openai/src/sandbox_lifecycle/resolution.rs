use secrecy::SecretString;
use serde_json::{Map, Value};

#[derive(Debug, Clone)]
pub struct SandboxToolParams {
    pub sandbox_provider: String,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
}

pub trait SandboxToolRegistry: Send + Sync {
    fn resolve(&self, sandbox_tool_name: &str) -> Option<Map<String, Value>>;
}

#[must_use]
pub fn resolve_sandbox_tool<Registry>(
    sandbox_tool_name: Option<&str>,
    registry: &Registry,
) -> Option<SandboxToolParams>
where
    Registry: SandboxToolRegistry,
{
    let resolved = registry.resolve(sandbox_tool_name?)?;
    Some(SandboxToolParams {
        sandbox_provider: resolved
            .get("sandbox_provider")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        api_key: resolved
            .get("api_key")
            .and_then(Value::as_str)
            .map(SecretString::from),
        api_base: resolved
            .get("api_base")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

include!("resolution/tests/cases.rs");
