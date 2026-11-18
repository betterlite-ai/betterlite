use super::phase::IdentityPhase;
use super::write_transactional::ResetWrite;

// LiteLLM drops spend:key:{token} for a key and spend:user:{user_id} for a user, once the write has committed.
#[must_use]
pub(crate) fn spend_counter_keys(phase: IdentityPhase, writes: &[ResetWrite]) -> Vec<String> {
    let prefix = match phase {
        IdentityPhase::Keys => "spend:key",
        IdentityPhase::Users => "spend:user",
    };
    writes
        .iter()
        .map(|write| format!("{prefix}:{}", write.identifier))
        .collect()
}

mod tests;
