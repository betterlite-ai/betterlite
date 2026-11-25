use serde::Deserialize;

#[path = "../helpers/core_ops/responses_ws_managed.rs"]
pub(crate) mod managed;

#[derive(Deserialize)]
#[serde(tag = "service", content = "input", deny_unknown_fields)]
pub(crate) enum ManagedInput {
    #[serde(rename = "managed_responses_turns")]
    ManagedResponsesTurns(managed::Input),
}
