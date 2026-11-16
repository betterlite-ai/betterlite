use std::collections::BTreeSet;

use uuid::Uuid;

use crate::guardrail_selection::gate::GuardrailSelection;

// The per-request inputs the hooks read from the proxy; LiteLLM reaches the same state through self and data.
pub struct HookContext<'context> {
    pub selection: &'context (dyn GuardrailSelection + Sync),
    pub suppressed: &'context BTreeSet<String>,
    pub new_id: fn() -> Uuid,
}
