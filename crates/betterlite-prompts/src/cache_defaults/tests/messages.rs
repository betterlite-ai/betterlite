use std::collections::BTreeMap;

use crate::arguments::json::{Json, Object};
use crate::cache_defaults::CachePolicy;
use crate::cache_defaults::DefaultsError;
use crate::cache_defaults::messages::{
    MessagesInjection, PlacedMessages, maybe_inject_cache_control,
};
use crate::cache_placement::system::SystemPrompt;
use crate::message_content::Message;

use super::support::{
    Facts, MARKED_SYSTEM, MODEL, OPENAI_MODEL, json, messages, object, system, wire,
};

const USER: &str = r#"[{"role": "user", "content": "hi"}]"#;
const MARKED_USER: &str = r#"[{
    "role": "user",
    "content": [{"type": "text", "text": "hi", "cache_control": {"type": "ephemeral"}}]
}]"#;
const CONFIGURED_USER: &str = r#"[
    {"location": "message", "role": "user", "index": null, "control": {"type": "ephemeral"}}
]"#;

struct Inject {
    model: Option<String>,
    messages: Vec<Message>,
    system: Option<SystemPrompt>,
    params: Object,
    tools: Option<Vec<Json>>,
    api_base: Option<String>,
    global_api_base: Option<String>,
    environment: BTreeMap<String, String>,
    global: bool,
    ttl: Option<String>,
}

impl Inject {
    fn claude() -> Self {
        Self {
            model: Some(MODEL.to_owned()),
            messages: messages(USER),
            system: Some(SystemPrompt::Text("be nice".to_owned())),
            params: Object::new(),
            tools: None,
            api_base: None,
            global_api_base: None,
            environment: BTreeMap::new(),
            global: true,
            ttl: None,
        }
    }
}

fn inject(injection: &mut Inject, facts: &Facts) -> Result<PlacedMessages, DefaultsError> {
    maybe_inject_cache_control(
        MessagesInjection {
            messages: &injection.messages,
            system: injection.system.as_ref(),
            params: &mut injection.params,
            model: injection.model.as_deref(),
            custom_llm_provider: None,
            tools: injection.tools.as_deref(),
            api_base: injection.api_base.as_deref(),
            global_api_base: injection.global_api_base.as_deref(),
            environment: &injection.environment,
            policy: CachePolicy {
                global_enabled: injection.global,
                ttl: injection.ttl.as_deref(),
            },
        },
        facts,
    )
}

fn run_injection(injection: &mut Inject, facts: &Facts) -> PlacedMessages {
    inject(injection, facts).unwrap()
}

#[test]
fn defaults_mark_the_system_prompt_and_the_trailing_turn() {
    let mut injection = Inject::claude();
    let placed = run_injection(&mut injection, &Facts::claude());
    assert_eq!(wire(&placed.messages), json(MARKED_USER));
    assert_eq!(
        wire(&placed.system),
        json(r#"[{"type": "text", "text": "be nice", "cache_control": {"type": "ephemeral"}}]"#)
    );
    assert_eq!(wire(&injection.params), json("{}"));
}

#[test]
fn defaults_do_not_apply_with_the_global_flag_off() {
    let mut injection = Inject {
        global: false,
        ..Inject::claude()
    };
    let placed = run_injection(&mut injection, &Facts::claude());
    assert_eq!(wire(&placed.messages), json(USER));
    assert_eq!(wire(&placed.system), json(r#""be nice""#));
}

#[test]
fn the_per_request_flag_is_read_from_the_params_and_removed() {
    let mut injection = Inject {
        global: false,
        params: object(r#"{"enable_prompt_caching": true}"#),
        ..Inject::claude()
    };
    let placed = run_injection(&mut injection, &Facts::claude());
    assert_eq!(wire(&placed.messages), json(MARKED_USER));
    assert_eq!(wire(&injection.params), json("{}"));
}

#[test]
fn a_null_per_request_flag_reads_as_unset() {
    let mut injection = Inject {
        global: true,
        params: object(r#"{"enable_prompt_caching": null}"#),
        ..Inject::claude()
    };
    let placed = run_injection(&mut injection, &Facts::claude());
    assert_eq!(wire(&placed.messages), json(MARKED_USER));
    assert_eq!(wire(&injection.params), json("{}"));
}

#[test]
fn a_non_boolean_per_request_flag_is_an_error() {
    let mut injection = Inject {
        params: object(r#"{"enable_prompt_caching": "yes"}"#),
        ..Inject::claude()
    };
    assert!(matches!(
        inject(&mut injection, &Facts::claude()),
        Err(DefaultsError::EnablePromptCaching)
    ));
}

#[test]
fn the_ttl_policy_reaches_the_default_marks() {
    let mut injection = Inject {
        ttl: Some("5m".to_owned()),
        ..Inject::claude()
    };
    let placed = run_injection(&mut injection, &Facts::claude());
    assert_eq!(
        wire(&placed.messages),
        json(
            r#"[{
            "role": "user",
            "content": [{"type": "text", "text": "hi", "cache_control": {"type": "ephemeral", "ttl": "5m"}}]
        }]"#
        )
    );
}

#[test]
fn configured_points_apply_over_client_marks() {
    let params = format!(
        r#"{{"cache_control_injection_points": {CONFIGURED_USER}, "cache_control": {{"type": "ephemeral"}}, "model_info": {{"id": "deployment-a"}}, "litellm_metadata": {{}}}}"#
    );
    let mut injection = Inject {
        params: object(&params),
        global: false,
        ..Inject::claude()
    };
    let placed = run_injection(&mut injection, &Facts::none());
    assert_eq!(wire(&placed.messages), json(MARKED_USER));
    assert_eq!(
        wire(&injection.params),
        json(
            r#"{"cache_control": {"type": "ephemeral"}, "model_info": {"id": "deployment-a"}, "litellm_metadata": {"litellm_gateway_injected_cache": "deployment-a"}}"#
        )
    );
}

#[test]
fn configured_points_are_removed_from_the_params() {
    let params = format!(r#"{{"cache_control_injection_points": {CONFIGURED_USER}}}"#);
    let mut injection = Inject {
        params: object(&params),
        ..Inject::claude()
    };
    let placed = run_injection(&mut injection, &Facts::none());
    assert_eq!(wire(&placed.messages), json(MARKED_USER));
    assert_eq!(wire(&injection.params), json("{}"));
}

#[test]
fn a_null_configured_list_falls_back_to_defaults() {
    let mut injection = Inject {
        params: object(r#"{"cache_control_injection_points": null}"#),
        ..Inject::claude()
    };
    let placed = run_injection(&mut injection, &Facts::claude());
    assert_eq!(wire(&placed.messages), json(MARKED_USER));
    assert_eq!(wire(&injection.params), json("{}"));
}

#[test]
fn a_tool_config_point_is_written_back_for_downstream_transforms() {
    let params = r#"{"cache_control_injection_points": [{"location": "tool_config", "control": {"type": "ephemeral"}}]}"#;
    let mut injection = Inject {
        params: object(params),
        ..Inject::claude()
    };
    let placed = run_injection(&mut injection, &Facts::none());
    assert_eq!(
        wire(&placed.messages),
        json(r#"[{"role": "user", "content": [{"type": "text", "text": "hi"}]}]"#)
    );
    assert_eq!(wire(&injection.params), json(params));
}

#[test]
fn a_tool_config_point_is_dropped_when_the_cap_is_reached() {
    let conversation = r#"[
        {"role": "user", "content": [{"type": "text", "text": "a", "cache_control": {"type": "ephemeral"}}]},
        {"role": "user", "content": [{"type": "text", "text": "b", "cache_control": {"type": "ephemeral"}}]},
        {"role": "user", "content": [{"type": "text", "text": "c", "cache_control": {"type": "ephemeral"}}]},
        {"role": "user", "content": [{"type": "text", "text": "d", "cache_control": {"type": "ephemeral"}}]}
    ]"#;
    let mut injection = Inject {
        messages: messages(conversation),
        system: None,
        params: object(
            r#"{"cache_control_injection_points": [{"location": "tool_config", "control": {"type": "ephemeral"}}]}"#,
        ),
        ..Inject::claude()
    };
    run_injection(&mut injection, &Facts::none());
    assert_eq!(wire(&injection.params), json("{}"));
}

#[test]
fn attribution_is_recorded_only_for_new_breakpoints() {
    let params = format!(
        r#"{{"cache_control_injection_points": {CONFIGURED_USER}, "model_info": {{"id": "deployment-a"}}, "metadata": {{}}}}"#
    );
    let mut injection = Inject {
        params: object(&params),
        ..Inject::claude()
    };
    run_injection(&mut injection, &Facts::none());
    assert_eq!(
        wire(&injection.params),
        json(
            r#"{"model_info": {"id": "deployment-a"}, "metadata": {"litellm_gateway_injected_cache": "deployment-a"}}"#
        )
    );
}

#[test]
fn attribution_leaves_a_marked_message_turn_uncredited() {
    let params = format!(
        r#"{{"cache_control_injection_points": {CONFIGURED_USER}, "model_info": {{"id": "deployment-a"}}, "metadata": {{}}}}"#
    );
    let mut injection = Inject {
        messages: messages(MARKED_USER),
        system: None,
        params: object(&params),
        ..Inject::claude()
    };
    let placed = run_injection(&mut injection, &Facts::none());
    assert_eq!(wire(&placed.messages), json(MARKED_USER));
    assert_eq!(
        wire(&injection.params),
        json(r#"{"model_info": {"id": "deployment-a"}, "metadata": {}}"#)
    );
}

#[test]
fn attribution_never_narrows_an_every_deployment_mark() {
    let params = format!(
        r#"{{"cache_control_injection_points": {CONFIGURED_USER}, "model_info": {{"id": "deployment-b"}}, "metadata": {{"litellm_gateway_injected_cache": ""}}}}"#
    );
    let mut injection = Inject {
        params: object(&params),
        ..Inject::claude()
    };
    run_injection(&mut injection, &Facts::none());
    assert_eq!(
        wire(&injection.params),
        json(
            r#"{"model_info": {"id": "deployment-b"}, "metadata": {"litellm_gateway_injected_cache": ""}}"#
        )
    );
}

#[test]
fn no_metadata_bucket_is_created() {
    let params = format!(
        r#"{{"cache_control_injection_points": {CONFIGURED_USER}, "model_info": {{"id": "deployment-a"}}}}"#
    );
    let mut injection = Inject {
        params: object(&params),
        ..Inject::claude()
    };
    run_injection(&mut injection, &Facts::none());
    assert_eq!(
        wire(&injection.params),
        json(r#"{"model_info": {"id": "deployment-a"}}"#)
    );
}

#[test]
fn the_openai_dialect_marks_blocks_and_sets_explicit_mode() {
    let params = r#"{"cache_control_injection_points": [{"location": "message", "role": "user", "index": null, "control": null}]}"#;
    let mut injection = Inject {
        model: Some(OPENAI_MODEL.to_owned()),
        messages: messages(
            r#"[{"role": "user", "content": "hi"}, {"role": "assistant", "content": "hello"}]"#,
        ),
        system: None,
        params: object(params),
        api_base: Some("https://api.openai.com/v1".to_owned()),
        ..Inject::claude()
    };
    let facts = Facts::routed(OPENAI_MODEL, "openai").with_flag(OPENAI_MODEL, true);
    let placed = run_injection(&mut injection, &facts);
    assert_eq!(
        wire(&placed.messages),
        json(
            r#"[
            {"role": "user", "content": [{"type": "text", "text": "hi", "prompt_cache_breakpoint": {"mode": "explicit"}}]},
            {"role": "assistant", "content": [{"type": "text", "text": "hello"}]}
        ]"#
        )
    );
    assert_eq!(
        wire(&injection.params),
        json(r#"{"prompt_cache_options": {"mode": "explicit"}}"#)
    );
}

#[test]
fn the_openai_dialect_is_not_stamped_when_nothing_was_added() {
    let params = r#"{"cache_control_injection_points": [{"location": "message", "role": "user", "index": 0, "control": null}]}"#;
    let mut injection = Inject {
        model: Some(OPENAI_MODEL.to_owned()),
        messages: messages(MARKED_USER),
        system: None,
        params: object(params),
        api_base: Some("https://api.openai.com/v1".to_owned()),
        ..Inject::claude()
    };
    let facts = Facts::routed(OPENAI_MODEL, "openai").with_flag(OPENAI_MODEL, true);
    run_injection(&mut injection, &facts);
    assert_eq!(wire(&injection.params), json("{}"));
}

#[test]
fn an_existing_prompt_cache_options_is_not_overwritten() {
    let params = r#"{"cache_control_injection_points": [{"location": "message", "role": "user", "index": 0, "control": null}], "prompt_cache_options": {"mode": "implicit"}}"#;
    let mut injection = Inject {
        model: Some(OPENAI_MODEL.to_owned()),
        messages: messages(r#"[{"role": "user", "content": "hi"}]"#),
        system: None,
        params: object(params),
        api_base: Some("https://api.openai.com/v1".to_owned()),
        ..Inject::claude()
    };
    let facts = Facts::routed(OPENAI_MODEL, "openai").with_flag(OPENAI_MODEL, true);
    run_injection(&mut injection, &facts);
    assert_eq!(
        wire(&injection.params),
        json(r#"{"prompt_cache_options": {"mode": "implicit"}}"#)
    );
}

#[test]
fn configured_points_apply_without_a_model_but_defaults_do_not() {
    let params = format!(r#"{{"cache_control_injection_points": {CONFIGURED_USER}}}"#);
    let mut injection = Inject {
        model: None,
        params: object(&params),
        ..Inject::claude()
    };
    let placed = run_injection(&mut injection, &Facts::none());
    assert_eq!(wire(&placed.messages), json(MARKED_USER));
    assert_eq!(wire(&injection.params), json("{}"));
    let mut quiet = Inject {
        model: None,
        ..Inject::claude()
    };
    let untouched = run_injection(&mut quiet, &Facts::claude());
    assert_eq!(wire(&untouched.messages), json(USER));
}

#[test]
fn client_marks_stand_the_defaults_down() {
    let mut injection = Inject {
        messages: messages(MARKED_USER),
        system: None,
        ..Inject::claude()
    };
    let placed = run_injection(&mut injection, &Facts::claude());
    assert_eq!(wire(&placed.messages), json(MARKED_USER));
    assert_eq!(wire(&injection.params), json("{}"));
}

#[test]
fn a_marked_system_block_is_not_marked_twice() {
    let mut injection = Inject {
        messages: messages(USER),
        system: system(MARKED_SYSTEM),
        ..Inject::claude()
    };
    let placed = run_injection(&mut injection, &Facts::claude());
    assert_eq!(wire(&placed.messages), json(USER));
    assert_eq!(wire(&placed.system), json(MARKED_SYSTEM));
    assert_eq!(wire(&injection.params), json("{}"));
}
