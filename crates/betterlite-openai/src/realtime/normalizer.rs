use betterlite_models::decisions::Json;

// LiteLLM's RealtimeEventNormalizer protocol; OpenAI's default is passthrough, so it installs none
pub trait RealtimeEventNormalizer: Send + Sync {
    fn should_drop(&self, event: &Json) -> bool;

    fn normalize(&self, event: Json) -> Json;

    fn patch_outgoing_session(&self, session: Json) -> Json;
}
