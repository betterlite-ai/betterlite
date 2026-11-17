use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::cache_defaults::CachePolicy;
use betterlite_prompts::cache_defaults::chat::{RouterProjection, messages_with_defaults};

use super::support::{
    ClaudeFacts, FakePins, MARKED_MESSAGES, cache_key, deployment, ids, json_messages, messages,
    request, request_kwargs, service, success,
};
use crate::prompt_affinity::json::messages_json;

const PLAIN_MESSAGES: &str =
    r#"[{"role":"system","content":"be brief"},{"role":"user","content":"why is the sky blue"}]"#;

pub(super) async fn run(name: &str) {
    match name {
        "a_success_event_pins_the_deployment_the_next_request_reuses" => {
            a_success_event_pins_the_deployment_the_next_request_reuses().await;
        }
        "a_pin_outside_the_pool_keeps_every_deployment" => {
            a_pin_outside_the_pool_keeps_every_deployment().await;
        }
        "a_prompt_below_the_minimum_never_reads_the_pin" => {
            a_prompt_below_the_minimum_never_reads_the_pin().await;
        }
        "a_priority_group_keeps_every_deployment" => {
            a_priority_group_keeps_every_deployment().await;
        }
        "a_target_order_keeps_every_deployment" => a_target_order_keeps_every_deployment().await,
        "without_messages_the_pool_is_untouched" => without_messages_the_pool_is_untouched().await,
        "another_call_type_never_pins" => another_call_type_never_pins().await,
        "a_success_event_without_a_model_id_never_pins" => {
            a_success_event_without_a_model_id_never_pins().await;
        }
        "a_success_event_without_messages_never_pins" => {
            a_success_event_without_messages_never_pins().await;
        }
        "default_breakpoints_are_injected_when_the_global_flag_is_on" => {
            default_breakpoints_are_injected_when_the_global_flag_is_on().await;
        }
        "request_kwargs_enable_prompt_caching_injects_without_the_flag" => {
            request_kwargs_enable_prompt_caching_injects_without_the_flag().await;
        }
        "tools_marks_suppress_the_default_breakpoints" => {
            tools_marks_suppress_the_default_breakpoints().await;
        }
        "request_kwargs_without_the_flag_do_not_inject" => {
            request_kwargs_without_the_flag_do_not_inject().await;
        }
        "a_message_level_mark_ignores_later_messages" => {
            a_message_level_mark_ignores_later_messages()
        }
        "a_message_key_order_never_changes_the_digest" => {
            a_message_key_order_never_changes_the_digest();
        }
        "only_the_blocks_up_to_the_last_mark_are_hashed" => {
            only_the_blocks_up_to_the_last_mark_are_hashed();
        }
        "cache_control_markers_never_change_the_digest" => {
            cache_control_markers_never_change_the_digest();
        }
        "tools_seed_the_prefix_digest" => tools_seed_the_prefix_digest(),
        _ => panic!("unknown case {name}"),
    }
}

async fn a_success_event_pins_the_deployment_the_next_request_reuses() {
    let pins = Arc::new(FakePins::default());
    let affinity = service(Arc::clone(&pins), 2000, false);
    let raw = json_messages(MARKED_MESSAGES);
    affinity
        .record_success(&success("acompletion", Some("dep-a"), Some(&raw)))
        .await
        .unwrap();
    let pool = [
        deployment("dep-a", "claude-min"),
        deployment("dep-b", "claude-min"),
    ];
    let typed = messages(MARKED_MESSAGES);
    let selected = affinity
        .filter_deployments(&request(&pool, Some(&typed)))
        .await
        .unwrap();
    assert_eq!(ids(&selected), ["dep-a"]);
    assert_eq!(pins.keys(), [cache_key(&raw, None).unwrap()]);
}

async fn a_pin_outside_the_pool_keeps_every_deployment() {
    let pins = Arc::new(FakePins::default());
    let affinity = service(Arc::clone(&pins), 2000, false);
    let raw = json_messages(MARKED_MESSAGES);
    affinity
        .record_success(&success("acompletion", Some("dep-c"), Some(&raw)))
        .await
        .unwrap();
    let pool = [
        deployment("dep-a", "claude-min"),
        deployment("dep-b", "claude-min"),
    ];
    let typed = messages(MARKED_MESSAGES);
    let selected = affinity
        .filter_deployments(&request(&pool, Some(&typed)))
        .await
        .unwrap();
    assert_eq!(ids(&selected), ["dep-a", "dep-b"]);
    assert_eq!(
        affinity.pin_for(Some(&raw), None).await.unwrap().as_deref(),
        Some("dep-c")
    );
}

async fn a_prompt_below_the_minimum_never_reads_the_pin() {
    let pins = Arc::new(FakePins::default());
    let raw = json_messages(MARKED_MESSAGES);
    pins.seed(
        &cache_key(&raw, None).unwrap(),
        super::support::pin("dep-a"),
    );
    let pool = [
        deployment("dep-a", "claude-min"),
        deployment("dep-b", "claude-min"),
    ];
    let typed = messages(MARKED_MESSAGES);
    let below = service(Arc::clone(&pins), 399, false);
    let selected = below
        .filter_deployments(&request(&pool, Some(&typed)))
        .await
        .unwrap();
    assert_eq!(ids(&selected), ["dep-a", "dep-b"]);
}

async fn a_priority_group_keeps_every_deployment() {
    keeps_every_deployment_with(true, None).await;
}

async fn a_target_order_keeps_every_deployment() {
    keeps_every_deployment_with(false, Some(2)).await;
}

async fn keeps_every_deployment_with(priority_group: bool, target_order: Option<i64>) {
    let pins = Arc::new(FakePins::default());
    let affinity = service(Arc::clone(&pins), 2000, false);
    let raw = json_messages(MARKED_MESSAGES);
    pins.seed(
        &cache_key(&raw, None).unwrap(),
        super::support::pin("dep-a"),
    );
    let pool = [
        deployment("dep-a", "claude-min"),
        deployment("dep-b", "claude-min"),
    ];
    let typed = messages(MARKED_MESSAGES);
    let mut selection = request(&pool, Some(&typed));
    selection.priority_group = priority_group;
    selection.target_order = target_order;
    let selected = affinity.filter_deployments(&selection).await.unwrap();
    assert_eq!(ids(&selected), ["dep-a", "dep-b"]);
}

async fn without_messages_the_pool_is_untouched() {
    let pins = Arc::new(FakePins::default());
    let affinity = service(Arc::clone(&pins), 2000, false);
    let pool = [
        deployment("dep-a", "claude-min"),
        deployment("dep-b", "claude-min"),
    ];
    let selected = affinity
        .filter_deployments(&request(&pool, None))
        .await
        .unwrap();
    assert_eq!(ids(&selected), ["dep-a", "dep-b"]);
}

async fn another_call_type_never_pins() {
    let pins = Arc::new(FakePins::default());
    let affinity = service(Arc::clone(&pins), 2000, false);
    let raw = json_messages(MARKED_MESSAGES);
    affinity
        .record_success(&success("embedding", Some("dep-a"), Some(&raw)))
        .await
        .unwrap();
    assert!(pins.keys().is_empty());
}

async fn a_success_event_without_a_model_id_never_pins() {
    never_pins(None, Some(&json_messages(MARKED_MESSAGES))).await;
}

async fn a_success_event_without_messages_never_pins() {
    never_pins(Some("dep-a"), None).await;
}

async fn never_pins(model_id: Option<&str>, messages: Option<&[Json]>) {
    let pins = Arc::new(FakePins::default());
    let affinity = service(Arc::clone(&pins), 2000, false);
    affinity
        .record_success(&success("anthropic_messages", model_id, messages))
        .await
        .unwrap();
    assert!(pins.keys().is_empty());
}

async fn default_breakpoints_are_injected_when_the_global_flag_is_on() {
    let injected = injected_json(PLAIN_MESSAGES, &Object::new(), None, true);
    assert!(cache_key(&injected, None).is_some());
    let pins = Arc::new(FakePins::default());
    let on = service(Arc::clone(&pins), 2000, true);
    on.record_success(&success("completion", Some("dep-a"), Some(&injected)))
        .await
        .unwrap();
    let pool = [
        deployment("dep-a", "claude-min"),
        deployment("dep-b", "claude-min"),
    ];
    let typed = messages(PLAIN_MESSAGES);
    let selected = on
        .filter_deployments(&request(&pool, Some(&typed)))
        .await
        .unwrap();
    assert_eq!(ids(&selected), ["dep-a"]);
    let off = service(Arc::clone(&pins), 2000, false);
    let untouched = off
        .filter_deployments(&request(&pool, Some(&typed)))
        .await
        .unwrap();
    assert_eq!(ids(&untouched), ["dep-a", "dep-b"]);
}

async fn request_kwargs_enable_prompt_caching_injects_without_the_flag() {
    let kwargs = request_kwargs(r#"{"enable_prompt_caching": true}"#);
    narrows_with_request_kwargs(false, &kwargs, Some(true), &["dep-a"]).await;
}

async fn tools_marks_suppress_the_default_breakpoints() {
    let kwargs = request_kwargs(
        r#"{"tools":[{"type":"function","function":{"name":"lookup"},"cache_control":{"type":"ephemeral"}}]}"#,
    );
    narrows_with_request_kwargs(true, &kwargs, Some(false), &["dep-a", "dep-b"]).await;
}

async fn request_kwargs_without_the_flag_do_not_inject() {
    narrows_with_request_kwargs(false, &Object::new(), Some(true), &["dep-a", "dep-b"]).await;
}

// the pin sits under the key the default breakpoints produce, so only an injected prompt reaches it
async fn narrows_with_request_kwargs(
    global_enabled: bool,
    kwargs: &Object,
    enable: Option<bool>,
    expected: &[&str],
) {
    let sent = injected_json(PLAIN_MESSAGES, kwargs, enable, global_enabled);
    assert!(cache_key(&sent, None).is_some());
    let store = Arc::new(FakePins::default());
    let check = service(Arc::clone(&store), 2000, global_enabled);
    check
        .record_success(&success("completion", Some("dep-a"), Some(&sent)))
        .await
        .unwrap();
    let pool = [
        deployment("dep-a", "claude-min"),
        deployment("dep-b", "claude-min"),
    ];
    let typed = messages(PLAIN_MESSAGES);
    let mut selection = request(&pool, Some(&typed));
    selection.request_kwargs = Some(kwargs);
    let kept = check.filter_deployments(&selection).await.unwrap();
    assert_eq!(ids(&kept), expected);
}

fn a_message_level_mark_ignores_later_messages() {
    let marked = json_messages(
        r#"[{"role":"user","content":"one","cache_control":{"type":"ephemeral"}},{"role":"assistant","content":"later"}]"#,
    );
    let alone =
        json_messages(r#"[{"role":"user","content":"one","cache_control":{"type":"ephemeral"}}]"#);
    let other =
        json_messages(r#"[{"role":"user","content":"two","cache_control":{"type":"ephemeral"}}]"#);
    assert_eq!(cache_key(&marked, None), cache_key(&alone, None));
    assert_ne!(cache_key(&marked, None), cache_key(&other, None));
}

fn a_message_key_order_never_changes_the_digest() {
    let ordered =
        json_messages(r#"[{"role":"user","content":"one","cache_control":{"type":"ephemeral"}}]"#);
    let reordered =
        json_messages(r#"[{"cache_control":{"type":"ephemeral"},"content":"one","role":"user"}]"#);
    assert_eq!(cache_key(&ordered, None), cache_key(&reordered, None));
    assert!(cache_key(&ordered, None).is_some());
}

fn only_the_blocks_up_to_the_last_mark_are_hashed() {
    let tail_one = json_messages(
        r#"[{"role":"user","content":[{"type":"text","text":"one"},{"type":"text","text":"two","cache_control":{"type":"ephemeral"}},{"type":"text","text":"tail one"}]}]"#,
    );
    let tail_two = json_messages(
        r#"[{"role":"user","content":[{"type":"text","text":"one"},{"type":"text","text":"two","cache_control":{"type":"ephemeral"}},{"type":"text","text":"tail two"}]}]"#,
    );
    let later = json_messages(
        r#"[{"role":"user","content":[{"type":"text","text":"one"},{"type":"text","text":"two"},{"type":"text","text":"tail one","cache_control":{"type":"ephemeral"}}]}]"#,
    );
    assert_eq!(cache_key(&tail_one, None), cache_key(&tail_two, None));
    assert_ne!(cache_key(&tail_one, None), cache_key(&later, None));
}

fn cache_control_markers_never_change_the_digest() {
    let everywhere = json_messages(
        r#"[{"role":"user","content":[{"type":"text","text":"one","cache_control":{"type":"ephemeral"}},{"type":"text","text":"two","cache_control":{"type":"ephemeral"}}]}]"#,
    );
    let only_last = json_messages(
        r#"[{"role":"user","content":[{"type":"text","text":"one"},{"type":"text","text":"two","cache_control":{"type":"ephemeral"}}]}]"#,
    );
    assert_eq!(cache_key(&everywhere, None), cache_key(&only_last, None));
    assert!(cache_key(&everywhere, None).is_some());
}

fn tools_seed_the_prefix_digest() {
    let raw = json_messages(MARKED_MESSAGES);
    let tools = json_messages(r#"[{"type":"function","function":{"name":"lookup"}}]"#);
    let no_tools = json_messages("[]");
    assert_ne!(cache_key(&raw, None), cache_key(&raw, Some(&tools)));
    assert_ne!(
        cache_key(&raw, Some(&tools)),
        cache_key(&raw, Some(&no_tools))
    );
    assert_ne!(cache_key(&raw, None), cache_key(&raw, Some(&no_tools)));
}

fn injected_json(
    source: &str,
    request_kwargs: &Object,
    enable_prompt_caching: Option<bool>,
    global_enabled: bool,
) -> Vec<Json> {
    let typed = messages(source);
    let models = vec!["claude-min".to_owned()];
    let aliases = BTreeMap::new();
    let projection = RouterProjection {
        messages: &typed,
        models: &models,
        tools: None,
        enable_prompt_caching,
        request_kwargs,
        alias_map: &aliases,
        policy: CachePolicy {
            global_enabled,
            ttl: None,
        },
    };
    messages_json(&messages_with_defaults(&projection, &ClaudeFacts))
}
