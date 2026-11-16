mod tests;

use betterlite_auth::agent_policy::AgentResponse;
use betterlite_guardrails::guardrail_registry::callback::GuardrailCallback;

use super::{RegistryReadThrough, Resync};

pub trait AgentLookup: Send + Sync {
    fn by_id_or_name(&self, key: &str) -> Option<AgentResponse>;
}

pub trait GuardrailLookup: Send + Sync {
    fn initialized(&self, guardrail_name: &str) -> Option<&GuardrailCallback>;
}

pub async fn get_agent_with_read_through<L, R>(
    lookup: &L,
    read_through: &RegistryReadThrough<R>,
    key: &str,
) -> Option<AgentResponse>
where
    L: AgentLookup,
    R: Resync,
{
    let cached = lookup.by_id_or_name(key);
    if cached.is_none() && read_through.attempt(key).await {
        return lookup.by_id_or_name(key);
    }
    cached
}

// ast-grep-ignore: no-long-names
pub async fn get_guardrail_with_read_through<'lookup, L, R>(
    lookup: &'lookup L,
    read_through: &RegistryReadThrough<R>,
    guardrail_name: &str,
) -> Option<&'lookup GuardrailCallback>
where
    L: GuardrailLookup,
    R: Resync,
{
    if let Some(callback) = lookup.initialized(guardrail_name) {
        return Some(callback);
    }
    if !read_through.attempt(guardrail_name).await {
        return None;
    }
    lookup.initialized(guardrail_name)
}
