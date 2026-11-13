use betterlite_prompts::arguments::json::Json;

use super::fixture::{RECORDS, World, body_of, capture, failure, field, json, transform};

const USER: &str = r#"[{"role":"user","content":"hello"}]"#;

#[test]
fn reasoning_alias_precedence() {
    alias_medium_maps();
    alias_budget_ladder();
    alias_existing_thinking_wins();
    alias_nonstring_and_none();
    alias_invalid_effort();
}

#[test]
fn alias_medium_maps() {
    let world = World::load(RECORDS);
    let sent = body_of(transform(
        &world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":4096,"reasoning_effort":"medium"}"#,
        false,
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
    let sent = body_of(transform(world, "claude-sonnet-4-5", USER, &params, false));
    field(&sent, "thinking").cloned().expect("a mapped budget")
}

#[test]
fn alias_budget_ladder() {
    let world = World::load(RECORDS);
    assert_eq!(
        budget_for(&world, "minimal"),
        json(r#"{"type":"enabled","budget_tokens":1024}"#)
    );
    assert_eq!(
        budget_for(&world, "low"),
        json(r#"{"type":"enabled","budget_tokens":1024}"#)
    );
    assert_eq!(
        budget_for(&world, "high"),
        json(r#"{"type":"enabled","budget_tokens":4096}"#)
    );
    assert_eq!(
        budget_for(&world, "xhigh"),
        json(r#"{"type":"enabled","budget_tokens":8192}"#)
    );
    assert_eq!(
        budget_for(&world, "max"),
        json(r#"{"type":"enabled","budget_tokens":16384}"#)
    );
}

#[test]
fn alias_existing_thinking_wins() {
    let world = World::load(RECORDS);
    let sent = body_of(transform(
        &world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":32768,"reasoning_effort":"medium","thinking":{"type":"enabled","budget_tokens":1500}}"#,
        false,
    ));
    assert_eq!(
        field(&sent, "thinking"),
        Some(&json(r#"{"type":"enabled","budget_tokens":1500}"#))
    );
}

#[test]
fn alias_nonstring_and_none() {
    let world = World::load(RECORDS);
    let unmapped = body_of(transform(
        &world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":4096,"reasoning_effort":3}"#,
        false,
    ));
    assert_eq!(field(&unmapped, "thinking"), None);
    assert_eq!(field(&unmapped, "reasoning_effort"), None);

    let cleared = body_of(transform(
        &world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":4096,"reasoning_effort":"none","thinking":{"type":"enabled","budget_tokens":2000},"output_config":{"format":{"type":"json_schema"}}}"#,
        false,
    ));
    assert_eq!(field(&cleared, "thinking"), None);
    assert_eq!(field(&cleared, "output_config"), None);
}

#[test]
fn alias_invalid_effort() {
    let world = World::load(RECORDS);
    let legacy = failure(transform(
        &world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":4096,"reasoning_effort":"extreme"}"#,
        false,
    ));
    assert_eq!(legacy.status_code(), 400);
    assert!(legacy.to_string().contains("Unmapped reasoning effort"));

    let adaptive = failure(transform(
        &world,
        "claude-opus-4-6",
        USER,
        r#"{"max_tokens":4096,"reasoning_effort":"extreme"}"#,
        false,
    ));
    assert_eq!(adaptive.status_code(), 400);
    assert!(adaptive.to_string().contains("Invalid reasoning_effort"));
}

#[test]
fn adaptive_output_effort() {
    adaptive_effort_levels();
    adaptive_effort_top_levels();
    adaptive_effort_existing_config();
    adaptive_effort_gate();
}

#[test]
fn adaptive_effort_levels() {
    let world = World::load(RECORDS);
    let minimal = body_of(transform(
        &world,
        "claude-opus-4-6",
        USER,
        r#"{"max_tokens":32768,"reasoning_effort":"minimal"}"#,
        false,
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
        &world,
        "claude-opus-4-6",
        USER,
        r#"{"max_tokens":32768,"reasoning_effort":"low"}"#,
        false,
    ));
    assert_eq!(
        field(&low, "output_config"),
        Some(&json(r#"{"effort":"low"}"#))
    );
    let medium = body_of(transform(
        &world,
        "claude-opus-4-6",
        USER,
        r#"{"max_tokens":32768,"reasoning_effort":"medium"}"#,
        false,
    ));
    assert_eq!(
        field(&medium, "output_config"),
        Some(&json(r#"{"effort":"medium"}"#))
    );
}

#[test]
fn adaptive_effort_top_levels() {
    let world = World::load(RECORDS);
    let high = body_of(transform(
        &world,
        "claude-opus-4-6",
        USER,
        r#"{"max_tokens":32768,"reasoning_effort":"high"}"#,
        false,
    ));
    assert_eq!(
        field(&high, "output_config"),
        Some(&json(r#"{"effort":"high"}"#))
    );
    let max = body_of(transform(
        &world,
        "claude-opus-4-6",
        USER,
        r#"{"max_tokens":32768,"reasoning_effort":"max"}"#,
        false,
    ));
    assert_eq!(
        field(&max, "output_config"),
        Some(&json(r#"{"effort":"max"}"#))
    );
}

#[test]
fn adaptive_effort_existing_config() {
    let world = World::load(RECORDS);
    let existing = body_of(transform(
        &world,
        "claude-opus-4-6",
        USER,
        r#"{"max_tokens":32768,"reasoning_effort":"high","output_config":{"effort":"low","format":{"type":"json_schema"}}}"#,
        false,
    ));
    assert_eq!(
        field(&existing, "output_config"),
        Some(&json(r#"{"effort":"low","format":{"type":"json_schema"}}"#))
    );

    let format_only = body_of(transform(
        &world,
        "claude-opus-4-6",
        USER,
        r#"{"max_tokens":32768,"reasoning_effort":"medium","output_config":{"format":{"type":"json_schema"}}}"#,
        false,
    ));
    assert_eq!(
        field(&format_only, "output_config"),
        Some(&json(
            r#"{"effort":"medium","format":{"type":"json_schema"}}"#
        ))
    );

    let plain = body_of(transform(
        &world,
        "claude-opus-4-6",
        USER,
        r#"{"max_tokens":32768,"reasoning_effort":"medium","output_config":"plain"}"#,
        false,
    ));
    assert_eq!(
        field(&plain, "output_config"),
        Some(&json(r#"{"effort":"medium"}"#))
    );
}

#[test]
fn adaptive_effort_gate() {
    let world = World::load(RECORDS);
    let xhigh = failure(transform(
        &world,
        "claude-opus-4-6",
        USER,
        r#"{"max_tokens":32768,"reasoning_effort":"xhigh"}"#,
        false,
    ));
    assert_eq!(xhigh.status_code(), 400);
    assert_eq!(
        xhigh.to_string(),
        "effort='xhigh' is not supported by this model. Got model: claude-opus-4-6"
    );

    let thinking_present = failure(transform(
        &world,
        "claude-opus-4-6",
        USER,
        r#"{"max_tokens":32768,"reasoning_effort":"xhigh","thinking":{"type":"enabled","budget_tokens":1500}}"#,
        false,
    ));
    assert_eq!(thinking_present.status_code(), 400);
}

#[test]
fn legacy_budget_fit() {
    budget_cap();
    budget_drop_keeps_existing();
    budget_missing();
}

#[test]
fn budget_cap() {
    let world = World::load(RECORDS);
    let capped = body_of(transform(
        &world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":1500,"reasoning_effort":"medium"}"#,
        false,
    ));
    assert_eq!(
        field(&capped, "thinking"),
        Some(&json(r#"{"type":"enabled","budget_tokens":1499}"#))
    );

    let boundary = body_of(transform(
        &world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":1025,"reasoning_effort":"medium"}"#,
        false,
    ));
    assert_eq!(
        field(&boundary, "thinking"),
        Some(&json(r#"{"type":"enabled","budget_tokens":1024}"#))
    );
}

#[test]
fn budget_drop_keeps_existing() {
    let world = World::load(RECORDS);
    let (_guard, logs) = capture();
    let unfitting = body_of(transform(
        &world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":1024,"reasoning_effort":"medium","thinking":{"type":"enabled","budget_tokens":1500},"output_config":{"format":{"type":"json_schema"}}}"#,
        false,
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
        &world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":0,"reasoning_effort":"medium"}"#,
        false,
    ));
    assert_eq!(field(&zero, "thinking"), None);
}

#[test]
fn budget_missing() {
    let world = World::load(RECORDS);
    let missing = failure(transform(
        &world,
        "claude-sonnet-4-5",
        USER,
        r#"{"reasoning_effort":"medium"}"#,
        false,
    ));
    assert_eq!(
        missing.to_string(),
        "max_tokens is required for Anthropic /v1/messages API"
    );
    assert_eq!(missing.status_code(), 400);

    let null = failure(transform(
        &world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":null,"reasoning_effort":"medium"}"#,
        false,
    ));
    assert_eq!(null.to_string(), missing.to_string());

    let early = failure(transform(
        &world,
        "claude-sonnet-4-5",
        USER,
        r#"{"reasoning_effort":"extreme"}"#,
        false,
    ));
    assert_eq!(early.to_string(), missing.to_string());
}

#[test]
fn nonadaptive_effort_translation() {
    legacy_thinking_for_nonadaptive();
    native_effort_kept();
    legacy_effort_fallbacks();
    invalid_legacy_effort();
}

#[test]
fn legacy_thinking_for_nonadaptive() {
    let world = World::load(RECORDS);
    let translated = body_of(transform(
        &world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":32768,"thinking":{"type":"adaptive"},"output_config":{"effort":"medium","format":{"type":"json_schema"}}}"#,
        false,
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
        &world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":32768,"thinking":{"type":"adaptive"}}"#,
        false,
    ));
    assert_eq!(
        field(&fallback, "thinking"),
        Some(&json(r#"{"type":"enabled","budget_tokens":2048}"#))
    );
}

#[test]
fn native_effort_kept() {
    let world = World::load(RECORDS);
    let native = body_of(transform(
        &world,
        "claude-opus-4-5",
        USER,
        r#"{"max_tokens":32768,"thinking":{"type":"adaptive"},"output_config":{"effort":"high"}}"#,
        false,
    ));
    assert_eq!(field(&native, "thinking"), None);
    assert_eq!(
        field(&native, "output_config"),
        Some(&json(r#"{"effort":"high"}"#))
    );

    let effort_only = body_of(transform(
        &world,
        "claude-opus-4-5",
        USER,
        r#"{"max_tokens":32768,"output_config":{"effort":"xhigh"}}"#,
        false,
    ));
    assert_eq!(field(&effort_only, "thinking"), None);
    assert_eq!(
        field(&effort_only, "output_config"),
        Some(&json(r#"{"effort":"xhigh"}"#))
    );
}

#[test]
fn legacy_effort_fallbacks() {
    let world = World::load(RECORDS);
    let capped = body_of(transform(
        &world,
        "claude-opus-4-5",
        USER,
        r#"{"max_tokens":32768,"thinking":{"type":"adaptive"},"output_config":{"effort":"xhigh"}}"#,
        false,
    ));
    assert_eq!(
        field(&capped, "thinking"),
        Some(&json(r#"{"type":"enabled","budget_tokens":8192}"#))
    );
    assert_eq!(field(&capped, "output_config"), None);

    let consumed = body_of(transform(
        &world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":32768,"thinking":{"type":"adaptive"},"output_config":{"effort":"medium"}}"#,
        false,
    ));
    assert_eq!(field(&consumed, "output_config"), None);

    let (_guard, logs) = capture();
    let unsupported = body_of(transform(
        &world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":1024,"thinking":{"type":"adaptive"},"output_config":{"effort":"medium","format":{"type":"json_schema"}}}"#,
        false,
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

#[test]
fn invalid_legacy_effort() {
    let world = World::load(RECORDS);
    let invalid = failure(transform(
        &world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":32768,"output_config":{"effort":"extreme"}}"#,
        false,
    ));
    assert_eq!(invalid.status_code(), 400);
    assert!(invalid.to_string().contains("Unmapped reasoning effort"));
}

#[test]
fn thinking_temperature_rules() {
    temperature_dropped();
    temperature_kept();
    temperature_after_translation();
}

#[test]
fn temperature_dropped() {
    let world = World::load(RECORDS);
    let enabled = body_of(transform(
        &world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":32768,"temperature":0,"thinking":{"type":"enabled","budget_tokens":2000}}"#,
        false,
    ));
    assert_eq!(field(&enabled, "temperature"), None);

    let effort = body_of(transform(
        &world,
        "claude-opus-4-5",
        USER,
        r#"{"max_tokens":32768,"temperature":0,"output_config":{"effort":"high"}}"#,
        false,
    ));
    assert_eq!(field(&effort, "temperature"), None);
}

#[test]
fn temperature_kept() {
    let world = World::load(RECORDS);
    let one = body_of(transform(
        &world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":32768,"temperature":1,"thinking":{"type":"enabled","budget_tokens":2000}}"#,
        false,
    ));
    assert_eq!(field(&one, "temperature"), Some(&json("1")));

    let null = body_of(transform(
        &world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":32768,"temperature":null,"thinking":{"type":"enabled","budget_tokens":2000}}"#,
        false,
    ));
    assert_eq!(field(&null, "temperature"), Some(&json("null")));

    let disabled = body_of(transform(
        &world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":32768,"temperature":0,"thinking":{"type":"disabled"}}"#,
        false,
    ));
    assert_eq!(field(&disabled, "temperature"), Some(&json("0")));

    let adaptive = body_of(transform(
        &world,
        "claude-opus-4-6",
        USER,
        r#"{"max_tokens":32768,"temperature":0,"reasoning_effort":"medium"}"#,
        false,
    ));
    assert_eq!(field(&adaptive, "temperature"), Some(&json("0")));
}

#[test]
fn temperature_after_translation() {
    let world = World::load(RECORDS);
    let translated = body_of(transform(
        &world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":32768,"temperature":0,"thinking":{"type":"adaptive"},"output_config":{"effort":"medium"}}"#,
        false,
    ));
    assert_eq!(
        field(&translated, "thinking"),
        Some(&json(r#"{"type":"enabled","budget_tokens":2048}"#))
    );
    assert_eq!(field(&translated, "temperature"), None);

    let dropped = body_of(transform(
        &world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":1024,"temperature":0,"thinking":{"type":"adaptive"},"output_config":{"effort":"medium"}}"#,
        false,
    ));
    assert_eq!(field(&dropped, "thinking"), None);
    assert_eq!(field(&dropped, "temperature"), Some(&json("0")));
}

#[test]
fn native_request_history() {
    history_retained();
    billing_history();
    context_history();
    advisor_history();
    encrypted_history();
    system_role_history();
    transformation_is_logged();
}

#[test]
fn history_retained() {
    let world = World::load(RECORDS);
    let sent = body_of(transform(
        &world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":2048,"temperature":1}"#,
        false,
    ));
    assert_eq!(field(&sent, "model"), Some(&json(r#""claude-sonnet-4-5""#)));
    assert_eq!(field(&sent, "max_tokens"), Some(&json("2048")));
    assert_eq!(field(&sent, "temperature"), Some(&json("1")));
    assert_eq!(
        field(&sent, "messages"),
        Some(&json(r#"[{"role":"user","content":"hello"}]"#))
    );
    assert_eq!(field(&sent, "system"), None);
    assert_eq!(field(&sent, "thinking"), None);
}

#[test]
fn billing_history() {
    let world = World::load(RECORDS);
    let system = r#"{"max_tokens":2048,"system":[{"type":"text","text":"x-anthropic-billing-header: cc"},{"type":"text","text":"keep"}]}"#;
    let kept = body_of(transform(&world, "claude-sonnet-4-5", USER, system, false));
    assert_eq!(
        field(&kept, "system"),
        Some(&json(
            r#"[{"type":"text","text":"x-anthropic-billing-header: cc"},{"type":"text","text":"keep"}]"#
        ))
    );
    let stripped = body_of(transform(&world, "claude-sonnet-4-5", USER, system, true));
    assert_eq!(
        field(&stripped, "system"),
        Some(&json(r#"[{"type":"text","text":"keep"}]"#))
    );
    let empty_text = body_of(transform(
        &world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":2048,"system":""}"#,
        true,
    ));
    assert_eq!(field(&empty_text, "system"), None);
    let empty_map = body_of(transform(
        &world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":2048,"system":{}}"#,
        true,
    ));
    assert_eq!(field(&empty_map, "system"), None);
}

#[test]
fn context_history() {
    let world = World::load(RECORDS);
    let context = body_of(transform(
        &world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":2048,"context_management":[{"type":"compaction","compact_threshold":200000}]}"#,
        false,
    ));
    assert_eq!(
        field(&context, "context_management"),
        Some(&json(
            r#"{"edits":[{"type":"compact_20260112","trigger":{"type":"input_tokens","value":200000}}]}"#
        ))
    );
    let unknown = body_of(transform(
        &world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":2048,"context_management":[{"type":"unknown","value":1}]}"#,
        false,
    ));
    assert_eq!(
        field(&unknown, "context_management"),
        Some(&json(r#"[{"type":"unknown","value":1}]"#))
    );
}

#[test]
fn advisor_history() {
    let world = World::load(RECORDS);
    let advisor = r#"[{"role":"assistant","content":[{"type":"server_tool_use","id":"a","name":"advisor","input":{}},{"type":"advisor_tool_result","tool_use_id":"a","content":"advice"},{"type":"text","text":"after"}]}]"#;
    let without_tool = body_of(transform(
        &world,
        "claude-sonnet-4-5",
        advisor,
        r#"{"max_tokens":2048}"#,
        false,
    ));
    assert_eq!(
        field(&without_tool, "messages"),
        Some(&json(
            r#"[{"role":"assistant","content":[{"type":"text","text":"after"}]}]"#
        ))
    );
    let with_tool = body_of(transform(
        &world,
        "claude-sonnet-4-5",
        advisor,
        r#"{"max_tokens":2048,"tools":[{"type":"advisor_20260301","name":"advisor"}]}"#,
        false,
    ));
    assert_eq!(
        field(&with_tool, "messages"),
        Some(&json(
            r#"[{"role":"assistant","content":[{"type":"server_tool_use","id":"a","name":"advisor","input":{}},{"type":"advisor_tool_result","tool_use_id":"a","content":"advice"},{"type":"text","text":"after"}]}]"#
        ))
    );
}

#[test]
fn encrypted_history() {
    let world = World::load(RECORDS);
    let encrypted = r#"[{"role":"assistant","content":[{"type":"thinking","thinking":"hidden","signature":"litellm_encrypted_reasoning:abc"},{"type":"text","text":"kept"}]},{"role":"assistant","content":[{"type":"redacted_thinking","data":"litellm_encrypted_reasoning:def"}]}]"#;
    let sent = body_of(transform(
        &world,
        "claude-sonnet-4-5",
        encrypted,
        r#"{"max_tokens":2048}"#,
        false,
    ));
    assert_eq!(
        field(&sent, "messages"),
        Some(&json(
            r#"[{"role":"assistant","content":[{"type":"text","text":"kept"}]}]"#
        ))
    );
}

#[test]
fn system_role_history() {
    let world = World::load(RECORDS);
    let messages = r#"[{"role":"system","content":"first"},{"role":"user","content":"hello"}]"#;
    let sent = body_of(transform(
        &world,
        "claude-sonnet-4-5",
        messages,
        r#"{"max_tokens":2048}"#,
        false,
    ));
    assert_eq!(
        field(&sent, "messages"),
        Some(&json(
            r#"[{"role":"system","content":"first"},{"role":"user","content":"hello"}]"#
        ))
    );
}

#[test]
fn transformation_is_logged() {
    let world = World::load(RECORDS);
    let (_guard, logs) = capture();
    let sent = body_of(transform(
        &world,
        "claude-sonnet-4-5",
        USER,
        r#"{"max_tokens":2048}"#,
        false,
    ));
    assert_eq!(field(&sent, "max_tokens"), Some(&json("2048")));
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
        false,
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
        false,
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
