use std::collections::BTreeSet;

use uuid::Uuid;

use crate::guardrail_selection::gate::GuardrailSelection;

pub struct HookContext<'context> {
    pub selection: &'context (dyn GuardrailSelection + Sync),
    pub suppressed: &'context BTreeSet<String>,
    pub new_id: fn() -> Uuid,
}
