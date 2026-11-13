use std::slice;

use betterlite_config::sections::guardrail_params::mode::{LitellmParamsMode, Mode, ModeTarget};

mod tests;

const MCP_EVENT_HOOKS: [&str; 3] = ["pre_mcp_call", "during_mcp_call", "post_mcp_call"];

const fn hook_names(target: &ModeTarget) -> &[String] {
    match target {
        ModeTarget::One(hook) => slice::from_ref(hook),
        ModeTarget::Many(hooks) => hooks.as_slice(),
    }
}

fn mcp_only(hooks: &[String]) -> bool {
    match hooks {
        [] => false,
        _ => hooks
            .iter()
            .all(|hook| MCP_EVENT_HOOKS.contains(&hook.as_str())),
    }
}

#[must_use]
pub fn configured_event_hooks(mode: &LitellmParamsMode) -> Vec<String> {
    match mode {
        LitellmParamsMode::One(hook) => vec![hook.clone()],
        LitellmParamsMode::Many(hooks) => hooks.clone(),
        LitellmParamsMode::Tagged(tagged) => tagged
            .tags
            .values()
            .chain(tagged.default.iter())
            .flat_map(hook_names)
            .cloned()
            .collect(),
    }
}

#[must_use]
pub fn is_mcp_only_mode(mode: &LitellmParamsMode) -> bool {
    match configured_event_hooks(mode).as_slice() {
        [] => false,
        hooks => mcp_only(hooks),
    }
}

fn output_hooks(target: &ModeTarget, include_mcp: bool) -> Vec<String> {
    let names = hook_names(target);
    // LiteLLM tests `not hooks` on the original str or list, so an empty string counts as empty
    let empty = match target {
        ModeTarget::One(hook) => hook.is_empty(),
        ModeTarget::Many(hooks) => hooks.is_empty(),
    };
    if empty || (!include_mcp && mcp_only(names)) {
        return Vec::new();
    }
    vec!["post_call".to_owned()]
}

#[must_use]
pub fn presidio_output_mode(mode: &LitellmParamsMode, include_mcp: bool) -> LitellmParamsMode {
    match mode {
        LitellmParamsMode::One(hook) => {
            LitellmParamsMode::Many(output_hooks(&ModeTarget::One(hook.clone()), include_mcp))
        }
        LitellmParamsMode::Many(hooks) => {
            LitellmParamsMode::Many(output_hooks(&ModeTarget::Many(hooks.clone()), include_mcp))
        }
        LitellmParamsMode::Tagged(tagged) => LitellmParamsMode::Tagged(Mode {
            tags: tagged
                .tags
                .iter()
                .map(|(tag, target)| {
                    (
                        tag.clone(),
                        ModeTarget::Many(output_hooks(target, include_mcp)),
                    )
                })
                .collect(),
            default: tagged
                .default
                .as_ref()
                .map(|target| ModeTarget::Many(output_hooks(target, include_mcp))),
        }),
    }
}
