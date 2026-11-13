use betterlite_prompts::cache_accounting::points::{
    CacheControlInjectionPoint, MessageInjectionPoint,
};
use betterlite_prompts::cache_placement::message_injections::apply_message_injections;
use betterlite_prompts::cache_placement::points::message_point;
use betterlite_prompts::message_content::Message;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Probe {
    messages: Vec<Message>,
    #[serde(default)]
    points: Vec<CacheControlInjectionPoint>,
    max_blocks: usize,
    #[serde(default)]
    openai_dialect: bool,
}

pub(super) fn result(probe: &Probe) -> Value {
    let points: Vec<&MessageInjectionPoint> =
        probe.points.iter().filter_map(message_point).collect();
    let processed = apply_message_injections(
        &points,
        &probe.messages,
        probe.max_blocks,
        probe.openai_dialect,
    );
    json!({ "messages": processed })
}
