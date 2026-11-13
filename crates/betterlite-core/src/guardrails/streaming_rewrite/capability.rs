use super::traits::{CallbackEntry, NamedGuardrail as _};

#[must_use]
pub fn find_guardrail_callback<'callbacks, Entry>(
    guardrail_name: &str,
    callbacks: &'callbacks [Entry],
) -> Option<&'callbacks Entry::Guardrail>
where
    Entry: CallbackEntry,
{
    for callback in callbacks {
        if let Some(guardrail) = callback.as_guardrail()
            && guardrail.guardrail_name() == Some(guardrail_name)
        {
            return Some(guardrail);
        }
    }
    None
}
