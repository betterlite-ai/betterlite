use betterlite_prompts::arguments::json::Json;

use super::fixture::{
    RECORDS, World, body_of, capture, failure, field, json, transform, transform_stripping,
};

const USER: &str = r#"[{"role":"user","content":"hello"}]"#;

#[test]
fn reasoning_alias_precedence() {
    let world = World::load(RECORDS);
    alias_medium_maps(&world);
    alias_budget_ladder(&world);
    alias_existing_thinking_wins(&world);
    alias_nonstring_and_none(&world);
    alias_invalid_effort(&world);
}

fn alias_medium_maps(world: &World) {
    let sent = body_of(transform(
        world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":4096,"reasoning_effort":"medium"}"#,
    ));
    assert_eq!(
        field(&sent, "thinking"),
        Some(&json(r#"{"type":"enabled","budget_tokens":2048}"#))
    );
    assert_eq!(field(&sent, "reasoning_effort"), None);
    assert_eq!(field(&sent, "max_tokens"), Some(&json("4096")));
    assert_eq!(field(&sent, "model"), Some(&json(r#""claude-sonnet-4-5""#)));
}

fn budget_for(world: &World, effort: &str) -> Json {
    let params = format!(r#"{{"max_tokens":32768,"reasoning_effort":"{effort}"}}"#);
    let sent = body_of(transform(world, "claude-sonnet-4-5", USER, &params));
    field(&sent, "thinking").cloned().expect("a mapped budget")
}

fn alias_budget_ladder(world: &World) {
    assert_eq!(
        budget_for(world, "minimal"),
        json(r#"{"type":"enabled","budget_tokens":1024}"#)
    );
    assert_eq!(
        budget_for(world, "low"),
        json(r#"{"type":"enabled","budget_tokens":1024}"#)
    );
    assert_eq!(
        budget_for(world, "high"),
        json(r#"{"type":"enabled","budget_tokens":4096}"#)
    );
    assert_eq!(
        budget_for(world, "xhigh"),
        json(r#"{"type":"enabled","budget_tokens":8192}"#)
    );
    assert_eq!(
        budget_for(world, "max"),
        json(r#"{"type":"enabled","budget_tokens":16384}"#)
    );
}

fn alias_existing_thinking_wins(world: &World) {
    let sent = body_of(transform(
        world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":32768,"reasoning_effort":"medium","thinking":{"type":"enabled","budget_tokens":1500}}"#,
    ));
    assert_eq!(
        field(&sent, "thinking"),
        Some(&json(r#"{"type":"enabled","budget_tokens":1500}"#))
    );
}

fn alias_nonstring_and_none(world: &World) {
    let unmapped = body_of(transform(
        world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":4096,"reasoning_effort":3}"#,
    ));
    assert_eq!(field(&unmapped, "thinking"), None);
    assert_eq!(field(&unmapped, "reasoning_effort"), None);

    let cleared = body_of(transform(
        world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":4096,"reasoning_effort":"none","thinking":{"type":"enabled","budget_tokens":2000},"output_config":{"format":{"type":"json_schema"}}}"#,
    ));
    assert_eq!(field(&cleared, "thinking"), None);
    assert_eq!(field(&cleared, "output_config"), None);
}

fn alias_invalid_effort(world: &World) {
    let legacy = failure(transform(
        world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":4096,"reasoning_effort":"extreme"}"#,
    ));
    assert_eq!(legacy.status_code(), 400);
    assert!(legacy.to_string().contains("Unmapped reasoning effort"));

    let adaptive = failure(transform(
        world,
        "claude-opus-4-6",
        USER,
        r#"{"max_tokens":4096,"reasoning_effort":"extreme"}"#,
    ));
    assert_eq!(adaptive.status_code(), 400);
    assert!(adaptive.to_string().contains("Invalid reasoning_effort"));
}

#[test]
fn adaptive_output_effort() {
    let world = World::load(RECORDS);
    adaptive_effort_levels(&world);
    adaptive_effort_top_levels(&world);
    adaptive_effort_existing_config(&world);
    adaptive_effort_gate(&world);
}

fn adaptive_effort_levels(world: &World) {
    let minimal = body_of(transform(
        world,
        "claude-opus-4-6",
        USER,
        r#"{"max_tokens":32768,"reasoning_effort":"minimal"}"#,
    ));
    assert_eq!(
        field(&minimal, "thinking"),
        Some(&json(r#"{"type":"adaptive","display":"summarized"}"#))
    );
    assert_eq!(
        field(&minimal, "output_config"),
        Some(&json(r#"{"effort":"low"}"#))
    );
    let low = body_of(transform(
        world,
        "claude-opus-4-6",
        USER,
        r#"{"max_tokens":32768,"reasoning_effort":"low"}"#,
    ));
    assert_eq!(
        field(&low, "output_config"),
        Some(&json(r#"{"effort":"low"}"#))
    );
    let medium = body_of(transform(
        world,
        "claude-opus-4-6",
        USER,
        r#"{"max_tokens":32768,"reasoning_effort":"medium"}"#,
    ));
    assert_eq!(
        field(&medium, "output_config"),
        Some(&json(r#"{"effort":"medium"}"#))
    );
}

fn adaptive_effort_top_levels(world: &World) {
    let high = body_of(transform(
        world,
        "claude-opus-4-6",
        USER,
        r#"{"max_tokens":32768,"reasoning_effort":"high"}"#,
    ));
    assert_eq!(
        field(&high, "output_config"),
        Some(&json(r#"{"effort":"high"}"#))
    );
    let max = body_of(transform(
        world,
        "claude-opus-4-6",
        USER,
        r#"{"max_tokens":32768,"reasoning_effort":"max"}"#,
    ));
    assert_eq!(
        field(&max, "output_config"),
        Some(&json(r#"{"effort":"max"}"#))
    );
}

fn adaptive_effort_existing_config(world: &World) {
    let existing = body_of(transform(
        world,
        "claude-opus-4-6",
        USER,
        r#"{"max_tokens":32768,"reasoning_effort":"high","output_config":{"effort":"low","format":{"type":"json_schema"}}}"#,
    ));
    assert_eq!(
        field(&existing, "output_config"),
        Some(&json(r#"{"effort":"low","format":{"type":"json_schema"}}"#))
    );

    let format_only = body_of(transform(
        world,
        "claude-opus-4-6",
        USER,
        r#"{"max_tokens":32768,"reasoning_effort":"medium","output_config":{"format":{"type":"json_schema"}}}"#,
    ));
    assert_eq!(
        field(&format_only, "output_config"),
        Some(&json(
            r#"{"effort":"medium","format":{"type":"json_schema"}}"#
        ))
    );

    let plain = body_of(transform(
        world,
        "claude-opus-4-6",
        USER,
        r#"{"max_tokens":32768,"reasoning_effort":"medium","output_config":"plain"}"#,
    ));
    assert_eq!(
        field(&plain, "output_config"),
        Some(&json(r#"{"effort":"medium"}"#))
    );
}

fn adaptive_effort_gate(world: &World) {
    let xhigh = failure(transform(
        world,
        "claude-opus-4-6",
        USER,
        r#"{"max_tokens":32768,"reasoning_effort":"xhigh"}"#,
    ));
    assert_eq!(xhigh.status_code(), 400);
    assert_eq!(
        xhigh.to_string(),
        "effort='xhigh' is not supported by this model. Got model: claude-opus-4-6"
    );

    let thinking_present = failure(transform(
        world,
        "claude-opus-4-6",
        USER,
        r#"{"max_tokens":32768,"reasoning_effort":"xhigh","thinking":{"type":"enabled","budget_tokens":1500}}"#,
    ));
    assert_eq!(thinking_present.status_code(), 400);
}

#[test]
fn legacy_budget_fit() {
    let world = World::load(RECORDS);
    budget_cap(&world);
    budget_drop_keeps_existing(&world);
    budget_missing(&world);
}

fn budget_cap(world: &World) {
    let capped = body_of(transform(
        world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":1500,"reasoning_effort":"medium"}"#,
    ));
    assert_eq!(
        field(&capped, "thinking"),
        Some(&json(r#"{"type":"enabled","budget_tokens":1499}"#))
    );

    let boundary = body_of(transform(
        world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":1025,"reasoning_effort":"medium"}"#,
    ));
    assert_eq!(
        field(&boundary, "thinking"),
        Some(&json(r#"{"type":"enabled","budget_tokens":1024}"#))
    );
}

fn budget_drop_keeps_existing(world: &World) {
    let (_guard, logs) = capture();
    let unfitting = body_of(transform(
        world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":1024,"reasoning_effort":"medium","thinking":{"type":"enabled","budget_tokens":1500},"output_config":{"format":{"type":"json_schema"}}}"#,
    ));
    assert_eq!(
        field(&unfitting, "thinking"),
        Some(&json(r#"{"type":"enabled","budget_tokens":1500}"#))
    );
    assert_eq!(
        field(&unfitting, "output_config"),
        Some(&json(r#"{"format":{"type":"json_schema"}}"#))
    );
    let warnings = logs.warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("Dropping `thinking` mapped from reasoning_effort=medium"),
        "{warnings:?}"
    );
    assert!(warnings[0].contains("max_tokens=1024"), "{warnings:?}");
    assert!(
        warnings[0].contains("model=claude-sonnet-4-5"),
        "{warnings:?}"
    );

    let zero = body_of(transform(
        world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":0,"reasoning_effort":"medium"}"#,
    ));
    assert_eq!(field(&zero, "thinking"), None);
}

fn budget_missing(world: &World) {
    let missing = failure(transform(
        world,
        "claude-sonnet-4-5",
        USER,
        r#"{"reasoning_effort":"medium"}"#,
    ));
    assert_eq!(
        missing.to_string(),
        "max_tokens is required for Anthropic /v1/messages API"
    );
    assert_eq!(missing.status_code(), 400);

    let null = failure(transform(
        world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":null,"reasoning_effort":"medium"}"#,
    ));
    assert_eq!(null.to_string(), missing.to_string());

    let early = failure(transform(
        world,
        "claude-sonnet-4-5",
        USER,
        r#"{"reasoning_effort":"extreme"}"#,
    ));
    assert_eq!(early.to_string(), missing.to_string());
}

#[test]
fn nonadaptive_effort_translation() {
    let world = World::load(RECORDS);
    legacy_thinking_for_nonadaptive(&world);
    native_effort_kept(&world);
    legacy_effort_fallbacks(&world);
    invalid_legacy_effort(&world);
}

fn legacy_thinking_for_nonadaptive(world: &World) {
    let translated = body_of(transform(
        world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":32768,"thinking":{"type":"adaptive"},"output_config":{"effort":"medium","format":{"type":"json_schema"}}}"#,
    ));
    assert_eq!(
        field(&translated, "thinking"),
        Some(&json(r#"{"type":"enabled","budget_tokens":2048}"#))
    );
    assert_eq!(
        field(&translated, "output_config"),
        Some(&json(r#"{"format":{"type":"json_schema"}}"#))
    );

    let fallback = body_of(transform(
        world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":32768,"thinking":{"type":"adaptive"}}"#,
    ));
    assert_eq!(
        field(&fallback, "thinking"),
        Some(&json(r#"{"type":"enabled","budget_tokens":2048}"#))
    );
}

fn native_effort_kept(world: &World) {
    let native = body_of(transform(
        world,
        "claude-opus-4-5",
        USER,
        r#"{"max_tokens":32768,"thinking":{"type":"adaptive"},"output_config":{"effort":"high"}}"#,
    ));
    assert_eq!(field(&native, "thinking"), None);
    assert_eq!(
        field(&native, "output_config"),
        Some(&json(r#"{"effort":"high"}"#))
    );

    let effort_only = body_of(transform(
        world,
        "claude-opus-4-5",
        USER,
        r#"{"max_tokens":32768,"output_config":{"effort":"xhigh"}}"#,
    ));
    assert_eq!(field(&effort_only, "thinking"), None);
    assert_eq!(
        field(&effort_only, "output_config"),
        Some(&json(r#"{"effort":"xhigh"}"#))
    );
}

fn legacy_effort_fallbacks(world: &World) {
    let capped = body_of(transform(
        world,
        "claude-opus-4-5",
        USER,
        r#"{"max_tokens":32768,"thinking":{"type":"adaptive"},"output_config":{"effort":"xhigh"}}"#,
    ));
    assert_eq!(
        field(&capped, "thinking"),
        Some(&json(r#"{"type":"enabled","budget_tokens":8192}"#))
    );
    assert_eq!(field(&capped, "output_config"), None);

    let consumed = body_of(transform(
        world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":32768,"thinking":{"type":"adaptive"},"output_config":{"effort":"medium"}}"#,
    ));
    assert_eq!(field(&consumed, "output_config"), None);

    let (_guard, logs) = capture();
    let unsupported = body_of(transform(
        world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":1024,"thinking":{"type":"adaptive"},"output_config":{"effort":"medium","format":{"type":"json_schema"}}}"#,
    ));
    assert_eq!(field(&unsupported, "thinking"), None);
    assert_eq!(
        field(&unsupported, "output_config"),
        Some(&json(r#"{"format":{"type":"json_schema"}}"#))
    );
    let warnings = logs.warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("Dropping adaptive `thinking`/`output_config.effort`"),
        "{warnings:?}"
    );
}

fn invalid_legacy_effort(world: &World) {
    let invalid = failure(transform(
        world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":32768,"output_config":{"effort":"extreme"}}"#,
    ));
    assert_eq!(invalid.status_code(), 400);
    assert!(invalid.to_string().contains("Unmapped reasoning effort"));
}

#[test]
fn thinking_temperature_rules() {
    let world = World::load(RECORDS);
    temperature_dropped(&world);
    temperature_kept(&world);
    temperature_after_translation(&world);
}

fn temperature_dropped(world: &World) {
    let enabled = body_of(transform(
        world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":32768,"temperature":0,"thinking":{"type":"enabled","budget_tokens":2000}}"#,
    ));
    assert_eq!(field(&enabled, "temperature"), None);

    let effort = body_of(transform(
        world,
        "claude-opus-4-5",
        USER,
        r#"{"max_tokens":32768,"temperature":0,"output_config":{"effort":"high"}}"#,
    ));
    assert_eq!(field(&effort, "temperature"), None);
}

fn temperature_kept(world: &World) {
    let one = body_of(transform(
        world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":32768,"temperature":1,"thinking":{"type":"enabled","budget_tokens":2000}}"#,
    ));
    assert_eq!(field(&one, "temperature"), Some(&json("1")));

    let null = body_of(transform(
        world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":32768,"temperature":null,"thinking":{"type":"enabled","budget_tokens":2000}}"#,
    ));
    assert_eq!(field(&null, "temperature"), Some(&json("null")));

    let disabled = body_of(transform(
        world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":32768,"temperature":0,"thinking":{"type":"disabled"}}"#,
    ));
    assert_eq!(field(&disabled, "temperature"), Some(&json("0")));

    let adaptive = body_of(transform(
        world,
        "claude-opus-4-6",
        USER,
        r#"{"max_tokens":32768,"temperature":0,"reasoning_effort":"medium"}"#,
    ));
    assert_eq!(field(&adaptive, "temperature"), Some(&json("0")));
}

fn temperature_after_translation(world: &World) {
    let translated = body_of(transform(
        world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":32768,"temperature":0,"thinking":{"type":"adaptive"},"output_config":{"effort":"medium"}}"#,
    ));
    assert_eq!(
        field(&translated, "thinking"),
        Some(&json(r#"{"type":"enabled","budget_tokens":2048}"#))
    );
    assert_eq!(field(&translated, "temperature"), None);

    let dropped = body_of(transform(
        world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":1024,"temperature":0,"thinking":{"type":"adaptive"},"output_config":{"effort":"medium"}}"#,
    ));
    assert_eq!(field(&dropped, "thinking"), None);
    assert_eq!(field(&dropped, "temperature"), Some(&json("0")));
}

#[test]
fn native_request_history() {
    let world = World::load(RECORDS);
    history_retained(&world);
    billing_history(&world);
    context_history(&world);
    advisor_history(&world);
    encrypted_history(&world);
    system_role_history(&world);
    transformation_is_logged(&world);
}

fn history_retained(world: &World) {
    let sent = body_of(transform(
        world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":2048,"temperature":1}"#,
    ));
    assert_eq!(field(&sent, "model"), Some(&json(r#""claude-sonnet-4-5""#)));
    assert_eq!(field(&sent, "max_tokens"), Some(&json("2048")));
    assert_eq!(field(&sent, "temperature"), Some(&json("1")));
    assert_eq!(
        field(&sent, "messages"),
        Some(&json(r#"[{"role":"user","content":"hello"}]"#))
    );
    assert_eq!(field(&sent, "system"), None);
}

fn billing_history(world: &World) {
    let system = r#"{"max_tokens":2048,"system":[{"type":"text","text":"x-anthropic-billing-header: cc"},{"type":"text","text":"keep"}]}"#;
    let kept = body_of(transform(world, "claude-sonnet-4-5", USER, system));
    assert_eq!(
        field(&kept, "system"),
        Some(&json(
            r#"[{"type":"text","text":"x-anthropic-billing-header: cc"},{"type":"text","text":"keep"}]"#
        ))
    );
    let stripped = body_of(transform_stripping(
        world,
        "claude-sonnet-4-5",
        USER,
        system,
    ));
    assert_eq!(
        field(&stripped, "system"),
        Some(&json(r#"[{"type":"text","text":"keep"}]"#))
    );
}

fn context_history(world: &World) {
    let context = body_of(transform(
        world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":2048,"context_management":[{"type":"compaction","compact_threshold":200000}]}"#,
    ));
    assert_eq!(
        field(&context, "context_management"),
        Some(&json(
            r#"{"edits":[{"type":"compact_20260112","trigger":{"type":"input_tokens","value":200000}}]}"#
        ))
    );
    let unknown = body_of(transform(
        world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":2048,"context_management":[{"type":"unknown","value":1}]}"#,
    ));
    assert_eq!(
        field(&unknown, "context_management"),
        Some(&json(r#"[{"type":"unknown","value":1}]"#))
    );
}

fn advisor_history(world: &World) {
    let advisor = r#"[{"role":"assistant","content":[{"type":"server_tool_use","id":"a","name":"advisor","input":{}},{"type":"advisor_tool_result","tool_use_id":"a","content":"advice"},{"type":"text","text":"after"}]}]"#;
    let without_tool = body_of(transform(
        world,
        "claude-sonnet-4-5",
        advisor,
        r#"{"max_tokens":2048}"#,
    ));
    assert_eq!(
        field(&without_tool, "messages"),
        Some(&json(
            r#"[{"role":"assistant","content":[{"type":"text","text":"after"}]}]"#
        ))
    );
    let with_tool = body_of(transform(
        world,
        "claude-sonnet-4-5",
        advisor,
        r#"{"max_tokens":2048,"tools":[{"type":"advisor_20260301","name":"advisor"}]}"#,
    ));
    assert_eq!(
        field(&with_tool, "messages"),
        Some(&json(
            r#"[{"role":"assistant","content":[{"type":"server_tool_use","id":"a","name":"advisor","input":{}},{"type":"advisor_tool_result","tool_use_id":"a","content":"advice"},{"type":"text","text":"after"}]}]"#
        ))
    );
}

fn encrypted_history(world: &World) {
    let encrypted = r#"[{"role":"assistant","content":[{"type":"thinking","thinking":"hidden","signature":"litellm_encrypted_reasoning:abc"},{"type":"text","text":"kept"}]},{"role":"assistant","content":[{"type":"redacted_thinking","data":"litellm_encrypted_reasoning:def"}]}]"#;
    let sent = body_of(transform(
        world,
        "claude-sonnet-4-5",
        encrypted,
        r#"{"max_tokens":2048}"#,
    ));
    assert_eq!(
        field(&sent, "messages"),
        Some(&json(
            r#"[{"role":"assistant","content":[{"type":"text","text":"kept"}]}]"#
        ))
    );
}

fn system_role_history(world: &World) {
    let messages = r#"[{"role":"system","content":"first"},{"role":"user","content":"hello"}]"#;
    let sent = body_of(transform(
        world,
        "claude-sonnet-4-5",
        messages,
        r#"{"max_tokens":2048}"#,
    ));
    assert_eq!(
        field(&sent, "messages"),
        Some(&json(
            r#"[{"role":"system","content":"first"},{"role":"user","content":"hello"}]"#
        ))
    );
}

fn transformation_is_logged(world: &World) {
    let (_guard, logs) = capture();
    let _unused = body_of(transform(
        world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":2048}"#,
    ));
    assert!(logs.logged_transformation());
}

#[test]
fn disabled_thinking_is_dropped_for_always_on_models() {
    let world = World::load(
        r#"{"claude-opus-4-6": {"litellm_provider": "anthropic", "supports_reasoning": true, "supports_adaptive_thinking": true, "supports_legacy_thinking": true, "thinking_always_on": true}}"#,
    );
    let (_guard, logs) = capture();
    let sent = body_of(transform(
        &world,
        "claude-opus-4-6",
        USER,
        r#"{"max_tokens":2048,"thinking":{"type":"disabled"}}"#,
    ));
    assert_eq!(field(&sent, "thinking"), None);
    assert_eq!(logs.warnings().len(), 1);
}

#[test]
fn legacy_thinking_is_translated_for_adaptive_models() {
    let world = World::load(
        r#"{"claude-opus-4-6": {"litellm_provider": "anthropic", "supports_reasoning": true, "supports_adaptive_thinking": true, "supports_legacy_thinking": false, "supports_max_reasoning_effort": true}}"#,
    );
    let sent = body_of(transform(
        &world,
        "claude-opus-4-6",
        USER,
        r#"{"max_tokens":32768,"thinking":{"type":"enabled","budget_tokens":5000,"display":"omitted"}}"#,
    ));
    assert_eq!(
        field(&sent, "thinking"),
        Some(&json(r#"{"type":"adaptive","display":"omitted"}"#))
    );
    assert_eq!(
        field(&sent, "output_config"),
        Some(&json(r#"{"effort":"high"}"#))
    );
}
