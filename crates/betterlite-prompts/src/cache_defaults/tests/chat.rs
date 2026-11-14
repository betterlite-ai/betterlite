use std::collections::BTreeMap;

use rstest::rstest;

use crate::arguments::json::{Json, Object};
use crate::cache_defaults::chat::{
    ChatSeeding, RouterProjection, messages_with_defaults, seed_default_injection_points,
};
use crate::cache_defaults::{CachePolicy, DefaultsError};
use crate::message_content::Message;

use super::support::{
    DEFAULT_POINTS, Facts, MARKED_CHAT_USER, MODEL, OPENAI_MODEL, json, messages, object, wire,
};

const USER: &str = r#"[{"role": "user", "content": "hi"}]"#;
const GROUP: &str = "group";
const CONFIGURED: &str = r#"[
    {"location": "message", "role": "user", "index": null, "control": {"type": "ephemeral"}}
]"#;

const fn policy(global: bool, ttl: Option<&str>) -> CachePolicy<'_> {
    CachePolicy {
        global_enabled: global,
        ttl,
    }
}

fn aliases(model: &str) -> BTreeMap<String, String> {
    BTreeMap::from([(GROUP.to_owned(), model.to_owned())])
}

fn project(conversation: &[Message], models: &[String], facts: &Facts) -> Vec<Message> {
    let aliases = aliases(MODEL);
    let kwargs = Object::new();
    let projection = RouterProjection {
        messages: conversation,
        models,
        tools: None,
        enable_prompt_caching: None,
        request_kwargs: &kwargs,
        alias_map: &aliases,
        policy: policy(true, None),
    };
    messages_with_defaults(&projection, facts)
}

#[test]
fn router_projection_applies_defaults_from_the_alias() {
    let conversation = messages(USER);
    let models = [GROUP.to_owned()];
    assert_eq!(
        wire(&project(&conversation, &models, &Facts::claude())),
        json(MARKED_CHAT_USER)
    );
}

#[test]
fn router_projection_uses_the_first_model_that_would_inject() {
    let conversation = messages(USER);
    let models = [OPENAI_MODEL.to_owned(), GROUP.to_owned()];
    assert_eq!(
        wire(&project(&conversation, &models, &Facts::claude())),
        json(MARKED_CHAT_USER)
    );
}

#[test]
fn router_projection_returns_the_input_when_no_model_would_inject() {
    let conversation = messages(USER);
    let models = [GROUP.to_owned()];
    assert_eq!(
        wire(&project(&conversation, &models, &Facts::none())),
        json(USER)
    );
}

#[test]
fn router_projection_stands_down_on_a_client_mark() {
    let conversation = messages(USER);
    let models = [GROUP.to_owned()];
    let aliases = aliases(MODEL);
    let kwargs = object(r#"{"cache_control": {"type": "ephemeral"}}"#);
    let projection = RouterProjection {
        messages: &conversation,
        models: &models,
        tools: None,
        enable_prompt_caching: None,
        request_kwargs: &kwargs,
        alias_map: &aliases,
        policy: policy(true, None),
    };
    assert_eq!(
        wire(&messages_with_defaults(&projection, &Facts::claude())),
        json(USER)
    );
}

#[test]
fn router_projection_stands_down_on_a_marked_system_argument() {
    let conversation = messages(USER);
    let models = [GROUP.to_owned()];
    let aliases = aliases(MODEL);
    let kwargs = object(
        r#"{"system": [{"type": "text", "text": "be nice", "cache_control": {"type": "ephemeral"}}]}"#,
    );
    let projection = RouterProjection {
        messages: &conversation,
        models: &models,
        tools: None,
        enable_prompt_caching: None,
        request_kwargs: &kwargs,
        alias_map: &aliases,
        policy: policy(true, None),
    };
    assert_eq!(
        wire(&messages_with_defaults(&projection, &Facts::claude())),
        json(USER)
    );
}

#[test]
fn router_projection_reads_the_one_shot_marker_from_the_request_kwargs() {
    let conversation = messages(USER);
    let models = [GROUP.to_owned()];
    let aliases = aliases(MODEL);
    let kwargs = object(
        r#"{"proxy_server_request": {"headers": {"User-Agent": "claude-cli/2.0.1"}}, "system": "x-anthropic-billing-header: cc_is_subagent=true"}"#,
    );
    let projection = RouterProjection {
        messages: &conversation,
        models: &models,
        tools: None,
        enable_prompt_caching: None,
        request_kwargs: &kwargs,
        alias_map: &aliases,
        policy: policy(true, None),
    };
    assert_eq!(
        wire(&messages_with_defaults(&projection, &Facts::claude())),
        json(USER)
    );
}

struct Seeding {
    model: String,
    params: Object,
    aliases: BTreeMap<String, String>,
    tools: Option<Vec<Json>>,
    enable: Option<bool>,
    api_base: Option<String>,
    global: bool,
    ttl: Option<String>,
}

impl Seeding {
    fn defaults() -> Self {
        Self {
            model: GROUP.to_owned(),
            params: object("{}"),
            aliases: aliases(MODEL),
            tools: None,
            enable: None,
            api_base: None,
            global: true,
            ttl: None,
        }
    }
}

fn seed(seeding: &mut Seeding, facts: &Facts) -> Result<(), DefaultsError> {
    let conversation = messages(USER);
    let environment = BTreeMap::new();
    seed_default_injection_points(
        ChatSeeding {
            model: &seeding.model,
            custom_llm_provider: None,
            messages: &conversation,
            params: &mut seeding.params,
            tools: seeding.tools.as_deref(),
            enable_prompt_caching: seeding.enable,
            api_base: seeding.api_base.as_deref(),
            global_api_base: None,
            environment: &environment,
            alias_map: &seeding.aliases,
            policy: policy(seeding.global, seeding.ttl.as_deref()),
        },
        facts,
    )
}

fn seeded_params(seeding: &mut Seeding, facts: &Facts) -> Object {
    seed(seeding, facts).unwrap();
    seeding.params.clone()
}

#[test]
fn defaults_are_seeded_into_the_injection_points_parameter() {
    let mut seeding = Seeding::defaults();
    let params = seeded_params(&mut seeding, &Facts::claude());
    let expected = format!(r#"{{"cache_control_injection_points": {DEFAULT_POINTS}}}"#);
    assert_eq!(wire(&params), json(&expected));
}

#[test]
fn a_per_request_flag_seeds_defaults_with_the_global_flag_off() {
    let mut seeding = Seeding {
        global: false,
        enable: Some(true),
        ..Seeding::defaults()
    };
    let params = seeded_params(&mut seeding, &Facts::claude());
    assert!(params.contains_key("cache_control_injection_points"));
}

#[test]
fn a_top_level_cache_control_keeps_the_seeding_from_defaulting() {
    let mut seeding = Seeding {
        params: object(r#"{"cache_control": {"type": "ephemeral"}}"#),
        ..Seeding::defaults()
    };
    let params = seeded_params(&mut seeding, &Facts::claude());
    assert_eq!(
        wire(&params),
        json(r#"{"cache_control": {"type": "ephemeral"}}"#)
    );
}

#[test]
fn an_extra_body_cache_control_keeps_the_seeding_from_defaulting() {
    let mut seeding = Seeding {
        params: object(r#"{"extra_body": {"cache_control": {"type": "ephemeral"}}}"#),
        ..Seeding::defaults()
    };
    let params = seeded_params(&mut seeding, &Facts::claude());
    assert_eq!(
        wire(&params),
        json(r#"{"extra_body": {"cache_control": {"type": "ephemeral"}}}"#)
    );
}

#[test]
fn no_defaults_are_seeded_when_the_capability_is_missing() {
    let mut seeding = Seeding::defaults();
    assert_eq!(
        wire(&seeded_params(&mut seeding, &Facts::none())),
        json("{}")
    );
}

#[test]
fn an_unresolved_alias_keeps_the_request_untouched() {
    let mut seeding = Seeding {
        aliases: BTreeMap::new(),
        ..Seeding::defaults()
    };
    assert_eq!(
        wire(&seeded_params(&mut seeding, &Facts::claude())),
        json("{}")
    );
}

#[test]
fn configured_points_are_stamped_with_the_external_breakpoint_count() {
    let params = format!(r#"{{"cache_control_injection_points": {CONFIGURED}}}"#);
    let mut seeding = Seeding {
        params: object(&params),
        tools: Some(vec![json(
            r#"{"type": "custom", "name": "t", "cache_control": {"type": "ephemeral"}}"#,
        )]),
        ..Seeding::defaults()
    };
    let seeded = seeded_params(&mut seeding, &Facts::none());
    let expected = r#"{"cache_control_injection_points": [{"location": "message", "role": "user", "index": null, "control": {"type": "ephemeral"}, "_litellm_external_breakpoints": 1}]}"#;
    assert_eq!(wire(&seeded), json(expected));
}

#[test]
fn configured_points_count_the_top_level_cache_control_as_external() {
    let params = format!(
        r#"{{"cache_control_injection_points": {CONFIGURED}, "cache_control": {{"type": "ephemeral"}}}}"#
    );
    let mut seeding = Seeding {
        params: object(&params),
        ..Seeding::defaults()
    };
    let seeded = seeded_params(&mut seeding, &Facts::none());
    let expected = r#"{"cache_control": {"type": "ephemeral"}, "cache_control_injection_points": [{"location": "message", "role": "user", "index": null, "control": {"type": "ephemeral"}, "_litellm_external_breakpoints": 1}]}"#;
    assert_eq!(wire(&seeded), json(expected));
}

#[test]
fn a_tool_search_tool_mark_is_not_counted() {
    let params = format!(r#"{{"cache_control_injection_points": {CONFIGURED}}}"#);
    let mut seeding = Seeding {
        params: object(&params),
        tools: Some(vec![json(
            r#"{"type": "tool_search_tool_regex_20251119", "name": "t", "cache_control": {"type": "ephemeral"}}"#,
        )]),
        ..Seeding::defaults()
    };
    let seeded = seeded_params(&mut seeding, &Facts::none());
    let expected = format!(r#"{{"cache_control_injection_points": {CONFIGURED}}}"#);
    assert_eq!(wire(&seeded), json(&expected));
}

#[test]
fn configured_points_keep_the_openai_dialect_stamp() {
    let params = format!(r#"{{"cache_control_injection_points": {CONFIGURED}}}"#);
    let mut seeding = Seeding {
        model: OPENAI_MODEL.to_owned(),
        params: object(&params),
        aliases: BTreeMap::new(),
        api_base: Some("https://api.openai.com/v1".to_owned()),
        ..Seeding::defaults()
    };
    let facts = Facts::routed(OPENAI_MODEL, "openai").with_flag(OPENAI_MODEL, true);
    let seeded = seeded_params(&mut seeding, &facts);
    let expected = r#"{"cache_control_injection_points": [{"location": "message", "role": "user", "index": null, "control": {"type": "ephemeral"}, "_litellm_openai_dialect": true}]}"#;
    assert_eq!(wire(&seeded), json(expected));
}

#[test]
fn configured_points_are_never_replaced_by_defaults() {
    let params = format!(r#"{{"cache_control_injection_points": {CONFIGURED}}}"#);
    let mut seeding = Seeding {
        params: object(&params),
        ..Seeding::defaults()
    };
    let seeded = seeded_params(&mut seeding, &Facts::claude());
    let expected = format!(r#"{{"cache_control_injection_points": {CONFIGURED}}}"#);
    assert_eq!(wire(&seeded), json(&expected));
}

#[rstest]
#[case::null("null")]
#[case::empty("[]")]
fn a_null_or_empty_configured_list_falls_back_to_defaults(#[case] configured: &str) {
    let params = format!(r#"{{"cache_control_injection_points": {configured}}}"#);
    let mut seeding = Seeding {
        params: object(&params),
        ..Seeding::defaults()
    };
    let seeded = seeded_params(&mut seeding, &Facts::claude());
    let expected = format!(r#"{{"cache_control_injection_points": {DEFAULT_POINTS}}}"#);
    assert_eq!(wire(&seeded), json(&expected));
}

#[test]
fn a_malformed_configured_list_is_an_error() {
    let mut seeding = Seeding {
        params: object(r#"{"cache_control_injection_points": "nope"}"#),
        ..Seeding::defaults()
    };
    assert!(matches!(
        seed(&mut seeding, &Facts::claude()),
        Err(DefaultsError::InjectionPoints { .. })
    ));
}
