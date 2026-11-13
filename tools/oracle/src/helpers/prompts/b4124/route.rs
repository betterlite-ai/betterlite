use betterlite_prompts::cache_accounting::points::CacheControlInjectionPoint;
use betterlite_prompts::cache_placement::messages_route::apply_messages_route;
use betterlite_prompts::cache_placement::system::SystemPrompt;
use betterlite_prompts::message_content::Message;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Probe {
    #[serde(default)]
    messages: Vec<Message>,
    #[serde(default)]
    system: Option<SystemPrompt>,
    #[serde(default)]
    points: Vec<CacheControlInjectionPoint>,
    #[serde(default)]
    openai_dialect: bool,
    #[serde(default)]
    external_breakpoints: usize,
}

pub(super) fn result(probe: &Probe) -> Value {
    let injection = apply_messages_route(
        &probe.messages,
        probe.system.as_ref(),
        &probe.points,
        probe.openai_dialect,
        probe.external_breakpoints,
    );
    json!({
        "messages": injection.messages,
        "system": injection.system,
        "remaining": injection.remaining,
    })
}
