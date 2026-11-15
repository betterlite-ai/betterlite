use betterlite_config::sections::anthropic_capability_catalog::Settings;
use tracing::Level;

use super::support::{Answers, Fixture, Session, Watch, map};
use crate::bedrock_model_capabilities::CapabilityError;
use crate::message_options::MessageOptionsError;
use crate::message_options::cache_ttl::remove_ttl_from_cache_control;
use crate::message_options::thinking::{inject_adaptive_thinking, resolve_clear_thinking_budget};
use crate::model_routes::{ParamMap, ParamValue};

const PREFIX: &str = "Bedrock clear_thinking_20251015:";

const MODEL_COST: &str = r#"{
    "bedrock/fixture-adaptive": {"litellm_provider": "bedrock", "supports_adaptive_thinking": true},
    "bedrock/fixture-legacy": {"litellm_provider": "bedrock", "mode": "chat"},
    "bedrock/fixture-weird-ceiling": {"litellm_provider": "bedrock", "supports_adaptive_thinking": true},
    "claude-4-5-fixture": {"cache_creation_input_token_cost_above_1hr": 0},
    "": {"cache_creation_input_token_cost_above_1hr": 1},
    "zero-entry": 0,
    "null-entry": null,
    "malformed": 5
}"#;

const BIG_REQUEST: &str = r#"{
    "max_tokens": 16,
    "tools": [
        {"name": "keep-both", "cache_control": {"type": "ephemeral", "scope": "global", "ttl": "1h", "custom": 1}},
        {"name": "keep-5m", "cache_control": {"ttl": "5m"}},
        {"name": "drop-10m", "cache_control": {"type": "ephemeral", "ttl": "10m"}},
        {"name": "drop-number", "cache_control": {"ttl": 1}},
        {"name": "non-dict-control", "cache_control": "ephemeral"},
        "not-a-tool",
        {"name": "no-cache-control"}
    ],
    "system": [
        {"type": "text", "text": "s", "cache_control": {"scope": "x", "ttl": "5m", "type": "ephemeral"}},
        "string-item",
        {"type": "text", "text": "plain", "cache_control": {"scope": "x"}}
    ],
    "messages": [
        {"role": "user", "content": [
            {"type": "text", "text": "hi", "cache_control": {"ttl": "1h", "scope": "global"}},
            {"type": "tool_result", "cache_control": {"ttl": "1h"}, "content": [{"type": "text", "cache_control": {"ttl": "1h", "scope": "global"}}]},
            {"cache_control": {"type": "ephemeral"}}
        ]},
        {"role": "assistant", "content": "plain"},
        {"role": "user"},
        "not-a-message"
    ]
}"#;

const BIG_EXPECTED: &str = r#"{
    "max_tokens": 16,
    "tools": [
        {"name": "keep-both", "cache_control": {"type": "ephemeral", "ttl": "1h", "custom": 1}},
        {"name": "keep-5m", "cache_control": {"ttl": "5m"}},
        {"name": "drop-10m", "cache_control": {"type": "ephemeral"}},
        {"name": "drop-number", "cache_control": {}},
        {"name": "non-dict-control", "cache_control": "ephemeral"},
        "not-a-tool",
        {"name": "no-cache-control"}
    ],
    "system": [
        {"type": "text", "text": "s", "cache_control": {"ttl": "5m", "type": "ephemeral"}},
        "string-item",
        {"type": "text", "text": "plain", "cache_control": {}}
    ],
    "messages": [
        {"role": "user", "content": [
            {"type": "text", "text": "hi", "cache_control": {"ttl": "1h"}},
            {"type": "tool_result", "cache_control": {"ttl": "1h"}, "content": [{"type": "text", "cache_control": {"ttl": "1h", "scope": "global"}}]},
            {"cache_control": {"type": "ephemeral"}}
        ]},
        {"role": "assistant", "content": "plain"},
        {"role": "user"},
        "not-a-message"
    ]
}"#;

const STRIPPED: &str = r#"{
    "tools": [{"cache_control": {"scope": "g", "ttl": "1h", "type": "ephemeral"}}],
    "system": [{"cache_control": {"ttl": "5m"}}],
    "messages": [{"content": [{"cache_control": {"ttl": "1h"}}]}]
}"#;

const STRIPPED_EXPECTED: &str = r#"{
    "tools": [{"cache_control": {"type": "ephemeral"}}],
    "system": [{"cache_control": {}}],
    "messages": [{"content": [{"cache_control": {}}]}]
}"#;

fn clearing(fields: &str) -> ParamMap {
    map(&format!(
        r#"{{"context_management": {{"edits": [{{"type": "clear_thinking_20251015"}}]}}, {fields}}}"#
    ))
}

fn ensure(
    session: &mut Session<'_>,
    budgets: &Settings,
    model: &str,
    request: ParamMap,
) -> (
    Result<bool, MessageOptionsError>,
    ParamMap,
    Vec<(Level, String)>,
) {
    let mut request = request;
    let watch = Watch::default();
    let guard = watch.install();
    let result = session.ensure_thinking(budgets, &mut request, model);
    let messages = watch.messages();
    drop(guard);
    (result, request, messages)
}

pub(super) fn cache_ttl_model_policy() {
    let model_cost = map(MODEL_COST);
    let watch = Watch::default();
    let _guard = watch.install();
    let mut request = map(BIG_REQUEST);
    remove_ttl_from_cache_control(&mut request, Some("claude-4-5-fixture"), &model_cost)
        .expect("a dict-shaped map reads");
    assert_eq!(request, map(BIG_EXPECTED));
    assert!(watch.messages().is_empty());

    let mut stripped = map(STRIPPED);
    remove_ttl_from_cache_control(&mut stripped, Some("unknown-model"), &model_cost)
        .expect("a dict-shaped map reads");
    assert_eq!(stripped, map(STRIPPED_EXPECTED));

    for model in [None, Some(""), Some("zero-entry"), Some("null-entry")] {
        let mut request = map(STRIPPED);
        remove_ttl_from_cache_control(&mut request, model, &model_cost)
            .expect("a falsey entry is no entry");
        assert_eq!(request, map(STRIPPED_EXPECTED), "model {model:?}");
    }

    let mut request = map(STRIPPED);
    let failure = remove_ttl_from_cache_control(&mut request, Some("malformed"), &model_cost);
    assert!(matches!(
        failure,
        Err(CapabilityError::MalformedEntry { .. })
    ));
    assert_eq!(request, map(STRIPPED));

    let containers =
        map(r#"{"tools": "abc", "system": {"type": "text"}, "messages": [{"content": "plain"}]}"#);
    let mut request = containers.clone();
    remove_ttl_from_cache_control(&mut request, Some("claude-4-5-fixture"), &model_cost)
        .expect("a dict-shaped map reads");
    assert_eq!(request, containers);
}

pub(super) fn clear_thinking_budget() {
    let fixture = Fixture::new(MODEL_COST);
    let mut session = fixture.session();
    let budgets = Settings::default();

    let (result, request, messages) = ensure(
        &mut session,
        &budgets,
        "fixture-adaptive",
        clearing(r#""thinking": {"type": "enabled", "budget_tokens": null}, "max_tokens": 1"#),
    );
    assert_eq!(result.ok(), Some(true));
    assert_eq!(
        request,
        clearing(
            r#""thinking": {"type": "adaptive"}, "output_config": {"effort": "low"}, "max_tokens": 1"#
        )
    );
    assert_eq!(
        messages,
        vec![(
            Level::DEBUG,
            format!(
                "{PREFIX} injected adaptive thinking with effort=low for model=fixture-adaptive"
            )
        )]
    );

    for (budget, effort) in [
        ("0", "low"),
        ("-10", "low"),
        ("1.9", "low"),
        ("true", "low"),
        ("2047", "low"),
        ("2048", "medium"),
        ("4095", "medium"),
        ("4096", "high"),
        ("8191", "high"),
        ("8192", "xhigh"),
        ("\"8192\"", "xhigh"),
    ] {
        let request = clearing(&format!(
            r#""thinking": {{"type": "enabled", "budget_tokens": {budget}}}, "max_tokens": 0"#
        ));
        let (result, request, messages) =
            ensure(&mut session, &budgets, "fixture-adaptive", request);
        assert_eq!(result.ok(), Some(true), "budget {budget}");
        assert_eq!(
            request,
            clearing(&format!(
                r#""thinking": {{"type": "adaptive"}}, "output_config": {{"effort": "{effort}"}}, "max_tokens": 0"#
            )),
            "budget {budget}"
        );
        assert_eq!(
            messages,
            vec![(
                Level::DEBUG,
                format!(
                    "{PREFIX} injected adaptive thinking with effort={effort} for model=fixture-adaptive"
                )
            )],
            "budget {budget}"
        );
    }

    let adaptive = clearing(r#""thinking": {"type": "adaptive", "display": "summarized"}"#);
    let (result, request, messages) =
        ensure(&mut session, &budgets, "fixture-adaptive", adaptive.clone());
    assert_eq!(result.ok(), Some(false));
    assert_eq!(request, adaptive);
    assert!(messages.is_empty());

    let non_adaptive = clearing(r#""thinking": {"type": "enabled", "budget_tokens": 5}"#);
    let (result, request, messages) = ensure(
        &mut session,
        &budgets,
        "fixture-opus-4-5",
        non_adaptive.clone(),
    );
    assert_eq!(result.ok(), Some(false));
    assert_eq!(request, non_adaptive);
    assert!(messages.is_empty());

    let unsupported = clearing(r#""thinking": {"type": "enabled", "budget_tokens": 5}"#);
    let (result, request, messages) = ensure(
        &mut session,
        &budgets,
        "fixture-legacy",
        unsupported.clone(),
    );
    assert_eq!(result.ok(), Some(false));
    assert_eq!(request, unsupported);
    assert!(messages.is_empty());

    for context_management in [
        "{}",
        r#"{"context_management": "plain"}"#,
        r#"{"context_management": {}}"#,
        r#"{"context_management": {"edits": {}}}"#,
        r#"{"context_management": {"edits": [{"type": "other"}, 7]}}"#,
        r#"{"context_management": {"edits": [{"type": 5}]}}"#,
    ] {
        let request = map(context_management);
        let (result, request, messages) =
            ensure(&mut session, &budgets, "fixture-adaptive", request.clone());
        assert_eq!(result.ok(), Some(false), "{context_management}");
        assert_eq!(request, map(context_management));
        assert!(messages.is_empty());
    }

    let mixed = map(
        r#"{"context_management": {"edits": [{"type": "other"}, {"type": "clear_thinking_20251015"}], "clear_tool_uses": true}, "thinking": {"type": "enabled", "budget_tokens": 2048}}"#,
    );
    let (result, request, _messages) = ensure(&mut session, &budgets, "fixture-adaptive", mixed);
    assert_eq!(result.ok(), Some(true));
    assert_eq!(
        request,
        map(
            r#"{"context_management": {"edits": [{"type": "other"}, {"type": "clear_thinking_20251015"}], "clear_tool_uses": true}, "thinking": {"type": "adaptive"}, "output_config": {"effort": "medium"}}"#
        )
    );

    let invalid = clearing(r#""thinking": {"type": "enabled", "budget_tokens": [1]}"#);
    let (result, request, messages) =
        ensure(&mut session, &budgets, "fixture-adaptive", invalid.clone());
    assert!(matches!(result, Err(MessageOptionsError::BudgetTokens)));
    assert_eq!(request, invalid);
    assert!(messages.is_empty());

    assert!(matches!(
        resolve_clear_thinking_budget(Some(&ParamValue::Number(serde_json::Number::from(7))), 1024),
        Ok(7)
    ));
    assert!(matches!(resolve_clear_thinking_budget(None, 5), Ok(5)));
}

pub(super) fn adaptive_clear_thinking() {
    let budgets = Settings::default();
    let custom = Settings {
        bedrock_min: 7,
        medium: 10,
        high: 20,
        xhigh: 30,
        ..Settings::default()
    };

    let request = map("{}");
    let watch = Watch::default();
    let guard = watch.install();
    let mut injected = request.clone();
    inject_adaptive_thinking(&mut injected, 1024, "fixture-adaptive", &budgets);
    assert_eq!(
        injected,
        map(r#"{"output_config": {"effort": "low"}, "thinking": {"type": "adaptive"}}"#)
    );
    assert_eq!(
        watch.messages(),
        vec![(
            Level::DEBUG,
            format!(
                "{PREFIX} injected adaptive thinking with effort=low for model=fixture-adaptive"
            )
        )]
    );
    drop(guard);

    let watch = Watch::default();
    let guard = watch.install();
    let mut kept = map(r#"{"output_config": {"effort": "max", "format": "x"}}"#);
    inject_adaptive_thinking(&mut kept, 1024, "fixture-adaptive", &budgets);
    assert_eq!(
        kept,
        map(
            r#"{"output_config": {"effort": "max", "format": "x"}, "thinking": {"type": "adaptive"}}"#
        )
    );
    assert_eq!(
        watch.messages(),
        vec![(
            Level::DEBUG,
            format!(
                "{PREFIX} injected adaptive thinking with effort=max for model=fixture-adaptive"
            )
        )]
    );
    drop(guard);

    let watch = Watch::default();
    let guard = watch.install();
    let mut empty = map(r#"{"output_config": {"effort": ""}, "thinking": {"type": "enabled"}}"#);
    inject_adaptive_thinking(&mut empty, 4096, "fixture-adaptive", &budgets);
    assert_eq!(
        empty,
        map(r#"{"output_config": {"effort": ""}, "thinking": {"type": "adaptive"}}"#)
    );
    assert_eq!(
        watch.messages(),
        vec![(
            Level::DEBUG,
            format!("{PREFIX} injected adaptive thinking with effort= for model=fixture-adaptive")
        )]
    );
    drop(guard);

    let watch = Watch::default();
    let guard = watch.install();
    let mut null = map(r#"{"output_config": {"effort": null}}"#);
    inject_adaptive_thinking(&mut null, 8192, "fixture-adaptive", &budgets);
    assert_eq!(
        null,
        map(r#"{"output_config": {"effort": null}, "thinking": {"type": "adaptive"}}"#)
    );
    assert_eq!(
        watch.messages(),
        vec![(
            Level::DEBUG,
            format!(
                "{PREFIX} injected adaptive thinking with effort=None for model=fixture-adaptive"
            )
        )]
    );
    drop(guard);

    let mut replaced = map(r#"{"output_config": "plain"}"#);
    inject_adaptive_thinking(&mut replaced, 8192, "fixture-adaptive", &budgets);
    assert_eq!(
        replaced,
        map(r#"{"output_config": {"effort": "xhigh"}, "thinking": {"type": "adaptive"}}"#)
    );

    let mut formatted = map(r#"{"output_config": {"format": "x"}}"#);
    inject_adaptive_thinking(&mut formatted, 2048, "fixture-adaptive", &budgets);
    assert_eq!(
        formatted,
        map(
            r#"{"output_config": {"format": "x", "effort": "medium"}, "thinking": {"type": "adaptive"}}"#
        )
    );

    for (thinking, expected) in [
        (
            r#"{"type": "enabled", "budget_tokens": 5, "display": ""}"#,
            r#"{"type": "adaptive", "display": ""}"#,
        ),
        (
            r#"{"type": "disabled", "display": false}"#,
            r#"{"type": "adaptive", "display": false}"#,
        ),
        (
            r#"{"type": "enabled", "display": null}"#,
            r#"{"type": "adaptive"}"#,
        ),
        (
            r#"{"type": "disabled", "display": "omitted", "effort": "high"}"#,
            r#"{"type": "adaptive", "display": "omitted"}"#,
        ),
        ("plain", r#"{"type": "adaptive"}"#),
    ] {
        let mut request = map(&format!(r#"{{"thinking": {thinking}}}"#));
        inject_adaptive_thinking(&mut request, 2048, "fixture-adaptive", &budgets);
        assert_eq!(
            request,
            map(&format!(
                r#"{{"thinking": {expected}, "output_config": {{"effort": "medium"}}}}"#
            )),
            "thinking {thinking}"
        );
    }

    for (budget, effort) in [
        (-1, "low"),
        (0, "low"),
        (2047, "low"),
        (2048, "medium"),
        (4095, "medium"),
        (4096, "high"),
        (8191, "high"),
        (8192, "xhigh"),
        (i64::MAX, "xhigh"),
    ] {
        let mut request = map("{}");
        inject_adaptive_thinking(&mut request, budget, "fixture-adaptive", &budgets);
        assert_eq!(
            request,
            map(&format!(
                r#"{{"output_config": {{"effort": "{effort}"}}, "thinking": {{"type": "adaptive"}}}}"#
            )),
            "budget {budget}"
        );
    }

    for (budget, effort) in [
        (9, "low"),
        (10, "medium"),
        (19, "medium"),
        (20, "high"),
        (29, "high"),
        (30, "xhigh"),
    ] {
        let mut request = map("{}");
        inject_adaptive_thinking(&mut request, budget, "fixture-adaptive", &custom);
        assert_eq!(
            request,
            map(&format!(
                r#"{{"output_config": {{"effort": "{effort}"}}, "thinking": {{"type": "adaptive"}}}}"#
            )),
            "budget {budget}"
        );
    }
}

pub(super) fn effort_clamp() {
    let fixture = Fixture::new(MODEL_COST);
    let mut session = fixture.session();
    let budgets = Settings::default();
    let answers = Answers::new(&[
        (
            "fixture-adaptive",
            r#"{"bedrock_output_config_effort_ceiling": "high"}"#,
        ),
        (
            "fixture-legacy",
            r#"{"bedrock_output_config_effort_ceiling": "high"}"#,
        ),
        (
            "fixture-weird-ceiling",
            r#"{"bedrock_output_config_effort_ceiling": "ultra"}"#,
        ),
    ]);

    for (effort, expected) in [
        ("xhigh", "high"),
        ("max", "high"),
        ("high", "high"),
        ("medium", "medium"),
        ("low", "low"),
    ] {
        let mut params = map(&format!(
            r#"{{"reasoning_effort": "{effort}", "temperature": 0.5}}"#
        ));
        let result = session.clamp(&budgets, "fixture-adaptive", &mut params, &answers);
        assert!(result.is_ok(), "effort {effort}");
        assert_eq!(
            params,
            map(&format!(
                r#"{{"reasoning_effort": "{expected}", "temperature": 0.5}}"#
            )),
            "effort {effort}"
        );
    }

    for params_json in [
        r#"{"reasoning_effort": "ultra"}"#,
        r#"{"reasoning_effort": 5}"#,
        r#"{"reasoning_effort": null}"#,
        r#"{"reasoning_effort": ["xhigh"]}"#,
        r#"{"reasoning_effort": true}"#,
        r#"{"reasoning_effort": {}}"#,
        r#"{"temperature": 0.5}"#,
    ] {
        let mut params = map(params_json);
        let result = session.clamp(&budgets, "fixture-adaptive", &mut params, &answers);
        assert!(result.is_ok(), "{params_json}");
        assert_eq!(params, map(params_json), "{params_json}");
    }

    let mut non_adaptive = map(r#"{"reasoning_effort": "xhigh"}"#);
    let result = session.clamp(&budgets, "fixture-legacy", &mut non_adaptive, &answers);
    assert!(result.is_ok());
    assert_eq!(non_adaptive, map(r#"{"reasoning_effort": "xhigh"}"#));

    let mut no_ceiling = map(r#"{"reasoning_effort": "xhigh"}"#);
    let result = session.clamp(
        &budgets,
        "fixture-adaptive",
        &mut no_ceiling,
        &Answers::new(&[]),
    );
    assert!(result.is_ok());
    assert_eq!(no_ceiling, map(r#"{"reasoning_effort": "xhigh"}"#));

    let mut weird = map(r#"{"reasoning_effort": "xhigh"}"#);
    let result = session.clamp(&budgets, "fixture-weird-ceiling", &mut weird, &answers);
    assert!(result.is_ok());
    assert_eq!(weird, map(r#"{"reasoning_effort": "xhigh"}"#));
}

pub(super) fn extended_thinking_support() {
    let fixture = Fixture::new(MODEL_COST);
    let mut session = fixture.session();
    let budgets = Settings::default();
    let models = [
        "fixture-adaptive",
        "fixture-legacy",
        "fixture-opus-4.5",
        "fixture-opus_4.5",
        "fixture-opus-4-5",
        "fixture-opus_4_5",
        "fixture-opus-4.1",
        "fixture-opus_4.1",
        "fixture-opus-4-1",
        "fixture-opus_4_1",
        "fixture-opus-4",
        "fixture-opus_4",
        "fixture-sonnet-4",
        "fixture-sonnet_4_20260101",
        "FIXTURE-OPUS-4-5",
        "fixture-opus-4-9-preview",
        "fixture-opus-5",
        "fixture-haiku-4-5",
        "fixture-sonnet-3-5",
        "mystery-model",
    ];
    let expected = [
        true, false, true, true, true, true, true, true, true, true, true, true, true, true, true,
        true, false, false, false, false,
    ];
    assert_eq!(session.extended_thinking(&budgets, &models), expected);
}

pub(super) fn clear_thinking_injection_limit() {
    let fixture = Fixture::new(MODEL_COST);
    let mut session = fixture.session();
    let budgets = Settings::default();

    let (result, request, messages) = ensure(
        &mut session,
        &budgets,
        "fixture-opus-4-5",
        clearing(r#""stream": true"#),
    );
    assert_eq!(result.ok(), Some(true));
    assert_eq!(
        request,
        clearing(r#""stream": true, "thinking": {"type": "enabled", "budget_tokens": 1024}"#)
    );
    assert_eq!(
        messages,
        vec![(
            Level::DEBUG,
            format!("{PREFIX} injected thinking with budget_tokens=1024")
        )]
    );

    for (max_tokens, logged) in [
        ("1024", "1024"),
        ("1023", "1023"),
        ("0", "0"),
        ("-5", "-5"),
        ("true", "True"),
        ("false", "False"),
    ] {
        let (result, request, messages) = ensure(
            &mut session,
            &budgets,
            "fixture-opus-4-5",
            clearing(&format!(r#""max_tokens": {max_tokens}"#)),
        );
        assert_eq!(result.ok(), Some(false), "max_tokens {max_tokens}");
        assert_eq!(request, clearing(&format!(r#""max_tokens": {max_tokens}"#)));
        assert_eq!(
            messages,
            vec![(
                Level::WARN,
                format!(
                    "{PREFIX} max_tokens={logged} is not greater than minimum thinking budget (1024); cannot inject thinking safely"
                )
            )],
            "max_tokens {max_tokens}"
        );
    }

    for max_tokens in ["1025", "\"4096\"", "4096.5", "null"] {
        let (result, request, _messages) = ensure(
            &mut session,
            &budgets,
            "fixture-opus-4-5",
            clearing(&format!(r#""max_tokens": {max_tokens}"#)),
        );
        assert_eq!(result.ok(), Some(true), "max_tokens {max_tokens}");
        let expected = clearing(&format!(
            r#""max_tokens": {max_tokens}, "thinking": {{"type": "enabled", "budget_tokens": 1024}}"#
        ));
        assert_eq!(request, expected, "max_tokens {max_tokens}");
    }

    let (result, request, messages) = ensure(
        &mut session,
        &budgets,
        "fixture-opus-4-5",
        clearing(r#""thinking": {}, "max_tokens": 1"#),
    );
    assert_eq!(result.ok(), Some(false));
    assert_eq!(request, clearing(r#""thinking": {}, "max_tokens": 1"#));
    assert_eq!(
        messages,
        vec![
            (
                Level::DEBUG,
                format!("{PREFIX} replacing thinking={{}} with minimal thinking config")
            ),
            (
                Level::WARN,
                format!(
                    "{PREFIX} max_tokens=1 is not greater than minimum thinking budget (1024); cannot inject thinking safely"
                )
            ),
        ]
    );

    let (result, request, messages) = ensure(
        &mut session,
        &budgets,
        "fixture-opus-4-5",
        clearing(r#""thinking": {}, "max_tokens": 5000"#),
    );
    assert_eq!(result.ok(), Some(true));
    assert_eq!(
        request,
        clearing(r#""thinking": {"type": "enabled", "budget_tokens": 1024}, "max_tokens": 5000"#)
    );
    assert_eq!(
        messages,
        vec![
            (
                Level::DEBUG,
                format!("{PREFIX} replacing thinking={{}} with minimal thinking config")
            ),
            (
                Level::DEBUG,
                format!("{PREFIX} injected thinking with budget_tokens=1024")
            ),
        ]
    );

    let tight = Settings {
        bedrock_min: 5,
        ..Settings::default()
    };
    let (result, _request, messages) = ensure(
        &mut session,
        &tight,
        "fixture-opus-4-5",
        clearing(r#""max_tokens": 5"#),
    );
    assert_eq!(result.ok(), Some(false));
    assert_eq!(
        messages,
        vec![(
            Level::WARN,
            format!(
                "{PREFIX} max_tokens=5 is not greater than minimum thinking budget (5); cannot inject thinking safely"
            )
        )]
    );
    let (result, request, messages) = ensure(
        &mut session,
        &tight,
        "fixture-opus-4-5",
        clearing(r#""max_tokens": 6"#),
    );
    assert_eq!(result.ok(), Some(true));
    assert_eq!(
        request,
        clearing(r#""max_tokens": 6, "thinking": {"type": "enabled", "budget_tokens": 5}"#)
    );
    assert_eq!(
        messages,
        vec![(
            Level::DEBUG,
            format!("{PREFIX} injected thinking with budget_tokens=5")
        )]
    );

    let (result, request, messages) = ensure(
        &mut session,
        &budgets,
        "fixture-adaptive",
        clearing(r#""max_tokens": 1"#),
    );
    assert_eq!(result.ok(), Some(false));
    assert_eq!(request, clearing(r#""max_tokens": 1"#));
    assert_eq!(
        messages,
        vec![(
            Level::WARN,
            format!(
                "{PREFIX} max_tokens=1 is not greater than minimum thinking budget (1024); cannot inject thinking safely"
            )
        )]
    );

    let (result, request, messages) = ensure(
        &mut session,
        &budgets,
        "fixture-adaptive",
        clearing(r#""max_tokens": 5000"#),
    );
    assert_eq!(result.ok(), Some(true));
    assert_eq!(
        request,
        clearing(
            r#""max_tokens": 5000, "output_config": {"effort": "low"}, "thinking": {"type": "adaptive"}"#
        )
    );
    assert_eq!(
        messages,
        vec![(
            Level::DEBUG,
            format!(
                "{PREFIX} injected adaptive thinking with effort=low for model=fixture-adaptive"
            )
        )]
    );

    let no_edit = map(
        r#"{"context_management": {"edits": [{"type": "other"}]}, "thinking": {}, "max_tokens": 1}"#,
    );
    let (result, request, messages) =
        ensure(&mut session, &budgets, "fixture-opus-4-5", no_edit.clone());
    assert_eq!(result.ok(), Some(false));
    assert_eq!(request, no_edit);
    assert!(messages.is_empty());

    let unsupported = clearing(r#""thinking": {}"#);
    let (result, request, messages) = ensure(
        &mut session,
        &budgets,
        "fixture-legacy",
        unsupported.clone(),
    );
    assert_eq!(result.ok(), Some(false));
    assert_eq!(request, unsupported);
    assert!(messages.is_empty());
}
