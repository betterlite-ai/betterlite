#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_params::mode::{LitellmParamsMode, Mode, ModeTarget};

use super::{configured_event_hooks, is_mcp_only_mode, presidio_output_mode};

fn one(hook: &str) -> ModeTarget {
    ModeTarget::One(hook.to_owned())
}

fn many(hooks: &[&str]) -> ModeTarget {
    ModeTarget::Many(hooks.iter().map(|hook| (*hook).to_owned()).collect())
}

fn tagged(tags: &[(&str, ModeTarget)], default: Option<ModeTarget>) -> LitellmParamsMode {
    LitellmParamsMode::Tagged(Mode {
        tags: tags
            .iter()
            .map(|(tag, target)| ((*tag).to_owned(), target.clone()))
            .collect::<BTreeMap<String, ModeTarget>>(),
        default,
    })
}

fn text(hook: &str) -> LitellmParamsMode {
    LitellmParamsMode::One(hook.to_owned())
}

fn list(hooks: &[&str]) -> LitellmParamsMode {
    LitellmParamsMode::Many(hooks.iter().map(|hook| (*hook).to_owned()).collect())
}

fn output(hooks: &[&str]) -> ModeTarget {
    many(hooks)
}

#[test]
fn configured_event_hooks_reads_a_text_a_list_and_a_tagged_mode() {
    assert_eq!(configured_event_hooks(&text("pre_call")), ["pre_call"]);
    assert_eq!(
        configured_event_hooks(&list(&["pre_call", "post_call"])),
        ["pre_call", "post_call"]
    );
    assert_eq!(
        configured_event_hooks(&tagged(
            &[
                ("chat", one("pre_call")),
                ("mcp", many(&["during_mcp_call", "post_mcp_call"]))
            ],
            Some(one("post_call")),
        )),
        ["pre_call", "during_mcp_call", "post_mcp_call", "post_call"]
    );
    assert_eq!(
        configured_event_hooks(&tagged(&[("mcp", one("pre_mcp_call"))], None)),
        ["pre_mcp_call"]
    );
}

#[test]
fn is_mcp_only_mode_needs_a_non_empty_all_mcp_hook_list() {
    assert!(is_mcp_only_mode(&text("pre_mcp_call")));
    assert!(is_mcp_only_mode(&text("during_mcp_call")));
    assert!(is_mcp_only_mode(&text("post_mcp_call")));
    assert!(is_mcp_only_mode(&list(&[
        "pre_mcp_call",
        "during_mcp_call",
        "post_mcp_call",
    ])));
    assert!(!is_mcp_only_mode(&list(&["pre_mcp_call", "pre_call"])));
    assert!(!is_mcp_only_mode(&list(&[])));
    assert!(!is_mcp_only_mode(&text("")));
    assert!(!is_mcp_only_mode(&text("pre_call")));
    assert!(is_mcp_only_mode(&tagged(
        &[("mcp", one("pre_mcp_call"))],
        Some(many(&["post_mcp_call"]))
    )));
    assert!(!is_mcp_only_mode(&tagged(
        &[("mcp", one("pre_mcp_call"))],
        Some(one("post_call"))
    )));
    assert!(!is_mcp_only_mode(&tagged(&[], None)));
}

#[test]
fn output_mode_maps_a_text_and_a_list_to_post_call() {
    assert_eq!(
        presidio_output_mode(&text("pre_call"), false),
        LitellmParamsMode::Many(vec!["post_call".to_owned()])
    );
    assert_eq!(
        presidio_output_mode(&list(&["pre_call", "post_call"]), false),
        LitellmParamsMode::Many(vec!["post_call".to_owned()])
    );
    assert_eq!(
        presidio_output_mode(&list(&[]), false),
        LitellmParamsMode::Many(Vec::new())
    );
    assert_eq!(
        presidio_output_mode(&text(""), false),
        LitellmParamsMode::Many(Vec::new())
    );
}

#[test]
fn output_mode_drops_mcp_only_hooks_unless_mcp_is_included() {
    assert_eq!(
        presidio_output_mode(&text("pre_mcp_call"), false),
        LitellmParamsMode::Many(Vec::new())
    );
    assert_eq!(
        presidio_output_mode(&text("pre_mcp_call"), true),
        LitellmParamsMode::Many(vec!["post_call".to_owned()])
    );
    assert_eq!(
        presidio_output_mode(&list(&["pre_mcp_call", "post_mcp_call"]), false),
        LitellmParamsMode::Many(Vec::new())
    );
    assert_eq!(
        presidio_output_mode(&list(&["pre_mcp_call", "post_mcp_call"]), true),
        LitellmParamsMode::Many(vec!["post_call".to_owned()])
    );
}

#[test]
fn output_mode_maps_every_tag_and_the_default_of_a_tagged_mode() {
    let mode = tagged(
        &[("chat", one("pre_call")), ("mcp", many(&["pre_mcp_call"]))],
        Some(one("post_call")),
    );
    let expected = tagged(
        &[("chat", output(&["post_call"])), ("mcp", output(&[]))],
        Some(output(&["post_call"])),
    );
    assert_eq!(presidio_output_mode(&mode, false), expected);
    let with_mcp = tagged(
        &[
            ("chat", output(&["post_call"])),
            ("mcp", output(&["post_call"])),
        ],
        Some(output(&["post_call"])),
    );
    assert_eq!(presidio_output_mode(&mode, true), with_mcp);
}

#[test]
fn output_mode_keeps_a_missing_tagged_default() {
    let mode = tagged(&[("chat", one("pre_call"))], None);
    let expected = tagged(&[("chat", output(&["post_call"]))], None);
    assert_eq!(presidio_output_mode(&mode, false), expected);
}
