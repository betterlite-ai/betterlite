pub mod error;

use rand::Rng;
use rand::seq::IndexedRandom as _;

use crate::guardrail_startup::router_list::RouterGuardrail;
use crate::router_guardrail_selection::error::GuardrailSelectionError;

mod tests;

// A router guardrail carries no weight, rpm or tpm, so LiteLLM's simple_shuffle ends in random.choice
pub fn select_router_guardrail<'list, R>(
    guardrails: &'list [RouterGuardrail],
    guardrail_name: &str,
    rng: &mut R,
) -> Result<&'list RouterGuardrail, GuardrailSelectionError>
where
    R: Rng,
{
    let matching: Vec<&RouterGuardrail> = guardrails
        .iter()
        .filter(|guardrail| guardrail.guardrail_name == guardrail_name)
        .collect();
    let not_found = || GuardrailSelectionError::NotFound {
        guardrail_name: guardrail_name.to_owned(),
    };
    match matching.as_slice() {
        [] => Err(not_found()),
        [only] => Ok(*only),
        several => several.choose(rng).copied().ok_or_else(not_found),
    }
}
