use super::events::HookPhase;

// LiteLLM's async_moderation_hook runs the pre-call scan under the during_call event types
pub(super) const PHASE: HookPhase = HookPhase::Moderation;

mod tests;
