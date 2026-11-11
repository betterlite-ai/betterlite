use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::cache_accounting::census::{
    RequestCacheMarks, count_cache_control_blocks, count_external_breakpoints,
    count_request_breakpoints, count_route_breakpoints, points_with_a_slot_left,
    request_has_cache_control, reserved_outside_messages,
};
use betterlite_prompts::cache_accounting::points::CacheControlInjectionPoint;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Probe {
    #[serde(default)]
    messages: Vec<Json>,
    #[serde(default)]
    system: Option<Json>,
    #[serde(default)]
    tools: Option<Vec<Json>>,
    #[serde(default)]
    cache_control: Option<Json>,
    #[serde(default)]
    request_kwargs: Option<Json>,
    #[serde(default)]
    points: Vec<CacheControlInjectionPoint>,
    #[serde(default)]
    external_breakpoints: usize,
    #[serde(default)]
    openai_dialect: bool,
    #[serde(default)]
    breakpoints_on_wire: usize,
    #[serde(default)]
    on_messages_route: bool,
}

pub(crate) fn counts(probes: &[Probe]) -> Value {
    let mut results = Vec::with_capacity(probes.len());
    for probe in probes {
        results.push(probe_json(probe));
    }
    json!({ "probes": results })
}

fn probe_json(probe: &Probe) -> Value {
    let tools = probe.tools.as_deref();
    let marks = RequestCacheMarks {
        messages: &probe.messages,
        system: probe.system.as_ref(),
        tools,
        cache_control: probe.cache_control.as_ref(),
        request_kwargs: probe.request_kwargs.as_ref(),
    };
    json!({
        "request_breakpoints": count_request_breakpoints(&probe.messages, probe.system.as_ref()),
        "external_breakpoints": count_external_breakpoints(
            tools,
            probe.cache_control.as_ref(),
            probe.request_kwargs.as_ref(),
        ),
        "route_breakpoints": count_route_breakpoints(
            tools,
            probe.cache_control.as_ref(),
            probe.request_kwargs.as_ref(),
        ),
        "reserved_blocks": reserved_outside_messages(
            &probe.points,
            probe.external_breakpoints,
            probe.openai_dialect,
        ),
        "points_with_slot": points_with_a_slot_left(
            &probe.points,
            probe.breakpoints_on_wire,
            probe.openai_dialect,
        ),
        "message_blocks": probe.messages.iter().map(count_cache_control_blocks).collect::<Vec<usize>>(),
        "message_has_control": probe.messages.iter().map(|message| count_cache_control_blocks(message) > 0).collect::<Vec<bool>>(),
        "request_has_cache_control": request_has_cache_control(&marks, probe.on_messages_route),
    })
}
