use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;
use betterlite_error::normalize::class::Class;
use betterlite_error::normalize::{ErrorFacts, Normalizer};
use betterlite_models::decisions::Json;

use super::fixture::{Fixture, Session, capture_warnings};
use crate::anthropic_capability_catalog::capability::supports_model_capability;
use crate::anthropic_capability_catalog::forced_tool::{
    DROP_FORCED_TOOL_WARNING, ForcedToolChoiceError, apply_forced_tool_choice,
    forced_tool_use_downgraded, forced_tool_use_unsupported,
};
use crate::anthropic_capability_catalog::sampling::SamplingParam::{Temperature, TopK, TopP};
use crate::anthropic_capability_catalog::sampling::{
    SamplingError, SamplingParam, SamplingRequest, apply_sampling_param, supports_sampling_params,
};
use crate::anthropic_capability_catalog::thinking::{
    DROP_DISABLED_THINKING_WARNING, EffortRequest, ThinkingError, ThinkingRequest, is_effort_used,
    legacy_budget_to_effort, maybe_drop_disabled_thinking, translate_legacy_thinking,
};
use crate::anthropic_capability_catalog::{
    CatalogSources, DropParams, EffortThresholds, OptionalParams,
};

const CATALOG_MODELS: &str = r#"{
    "anthropic/claude-x": {"litellm_provider": "anthropic", "supports_adaptive_thinking": true},
    "claude-y": {"litellm_provider": "anthropic", "supports_adaptive_thinking": false},
    "claude-z": {"litellm_provider": "anthropic"},
    "anthropic/claude-q": {"litellm_provider": "anthropic", "supports_adaptive_thinking": "yes"},
    "anthropic/claude-w": {"litellm_provider": "anthropic"},
    "claude-w": {"litellm_provider": "anthropic", "supports_adaptive_thinking": true}
}"#;
const CATALOG_PACKAGED: &str = r#"{
    "claude-x": {"thinking_always_on": true},
    "claude-y": {"supports_adaptive_thinking": true},
    "claude-z": {"supports_legacy_thinking": true},
    "claude-q": {"supports_adaptive_thinking": true}
}"#;

const THINKING_MODELS: &str = r#"{
    "anthropic/claude-opus-4-7": {"litellm_provider": "anthropic", "supports_adaptive_thinking": true},
    "claude-opus-4-6": {"litellm_provider": "anthropic", "supports_adaptive_thinking": true, "supports_legacy_thinking": true},
    "claude-sonnet-4-5": {"litellm_provider": "anthropic"}
}"#;

const FORCED_MODELS: &str = r#"{
    "claude-fable-5-1": {"litellm_provider": "anthropic", "supports_forced_tool_use": false},
    "claude-sonnet-4-5": {"litellm_provider": "anthropic"}
}"#;

const SAMPLING_MODELS: &str = r#"{
    "claude-opus-4-6": {"litellm_provider": "anthropic", "supports_sampling_params": true},
    "claude-opus-4-7": {"litellm_provider": "anthropic", "supports_sampling_params": false},
    "claude-sonnet-4-5": {"litellm_provider": "anthropic"},
    "claude-fable-lite": {"litellm_provider": "anthropic", "supports_sampling_params": true}
}"#;

const EFFORT_MODELS: &str = r#"{
    "anthropic/claude-opus-4-6": {"litellm_provider": "anthropic", "supports_xhigh_reasoning_effort": true},
    "anthropic/claude-sonnet-4-6": {"litellm_provider": "anthropic"}
}"#;

const DISABLED_MODELS: &str = r#"{
    "anthropic/claude-fable-5": {"litellm_provider": "anthropic", "thinking_always_on": true},
    "claude-sonnet-4-5": {"litellm_provider": "anthropic"}
}"#;

const ADAPTIVE_MODELS: &str = r#"{
    "anthropic/claude-opus-4-7": {"litellm_provider": "anthropic", "supports_adaptive_thinking": true}
}"#;

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("json value")
}

fn params(text: &str) -> OptionalParams {
    serde_json::from_str(text).expect("optional params")
}

async fn thinking_capabilities(fixture: &Fixture, model: &str) -> [bool; 3] {
    let mut session = Session::new(fixture);
    let catalogs = fixture.catalogs();
    let adaptive = supports_model_capability(
        model,
        "supports_adaptive_thinking",
        "anthropic",
        &mut session.environment(),
        &catalogs,
    )
    .await;
    let always = supports_model_capability(
        model,
        "thinking_always_on",
        "anthropic",
        &mut session.environment(),
        &catalogs,
    )
    .await;
    let legacy = supports_model_capability(
        model,
        "supports_legacy_thinking",
        "anthropic",
        &mut session.environment(),
        &catalogs,
    )
    .await;
    [adaptive, always, legacy]
}

async fn translate(
    fixture: &Fixture,
    model: &str,
    params: &mut OptionalParams,
) -> Result<(), ThinkingError> {
    let mut session = Session::new(fixture);
    let catalogs = fixture.catalogs();
    let request = ThinkingRequest {
        model,
        custom_llm_provider: "anthropic",
        thresholds: EffortThresholds::default(),
    };
    translate_legacy_thinking(&request, params, &mut session.environment(), &catalogs).await
}

async fn drop_disabled(fixture: &Fixture, model: &str, params: &mut OptionalParams) -> bool {
    let mut session = Session::new(fixture);
    let catalogs = fixture.catalogs();
    let mut environment = session.environment();
    maybe_drop_disabled_thinking(model, params, "anthropic", &mut environment, &catalogs).await
}

async fn effort(
    fixture: &Fixture,
    model: &str,
    budget_tokens: i64,
    thresholds: EffortThresholds,
) -> &'static str {
    let mut session = Session::new(fixture);
    let catalogs = fixture.catalogs();
    let request = EffortRequest {
        model,
        budget_tokens,
        custom_llm_provider: "anthropic",
        thresholds,
    };
    legacy_budget_to_effort(&request, &mut session.environment(), &catalogs).await
}

async fn effort_used(fixture: &Fixture, model: &str, params: Option<&OptionalParams>) -> bool {
    let mut session = Session::new(fixture);
    is_effort_used(
        params,
        model,
        "anthropic",
        &mut session.environment(),
        &fixture.catalogs(),
    )
    .await
}

fn sampling(
    fixture: &Fixture,
    model: &str,
    param: SamplingParam,
    value: &Json,
    drop: DropParams,
) -> Result<Option<Json>, SamplingError> {
    apply_sampling_param(
        &SamplingRequest {
            model,
            param,
            value,
            drop,
        },
        &fixture.catalogs(),
    )
}

pub(super) async fn provider_qualified_catalog() {
    let fixture = Fixture::new(CATALOG_MODELS, CATALOG_PACKAGED);
    assert_eq!(
        thinking_capabilities(&fixture, "claude-x").await,
        [true, true, false],
        "the resolved flag wins, then the raw packaged map answers"
    );
    assert_eq!(
        thinking_capabilities(&fixture, "claude-y").await,
        [false, false, false],
        "a resolved false is authoritative over the raw true"
    );
    assert_eq!(
        thinking_capabilities(&fixture, "claude-z").await,
        [false, false, true],
        "the raw fallback is the last resort"
    );
    assert_eq!(
        thinking_capabilities(&fixture, "claude-q").await,
        [true, false, false],
        "a non-bool resolved flag has no opinion, so the raw packaged true answers"
    );
    // The resolved anthropic/claude-w entry lacks the key and the raw walk never tries bare claude-w
    assert_eq!(
        thinking_capabilities(&fixture, "anthropic/claude-w").await,
        [true, false, false],
        "only _supports_factory's bare-entry fallback answers true"
    );
}

// Each row is the model, the optional_params before translation and the expected params after it.
const TRANSLATIONS: [(&str, &str, &str); 10] = [
    (
        "claude-opus-4-7",
        r#"{"thinking": {"type": "enabled", "budget_tokens": 5000, "display": "summarized"}}"#,
        r#"{"thinking": {"type": "adaptive", "display": "summarized"}, "output_config": {"effort": "high"}}"#,
    ),
    (
        "claude-opus-4-7",
        r#"{"thinking": {"type": "enabled", "budget_tokens": 4096, "display": "omitted"}}"#,
        r#"{"thinking": {"type": "adaptive", "display": "omitted"}, "output_config": {"effort": "high"}}"#,
    ),
    (
        "claude-opus-4-7",
        r#"{"thinking": {"type": "enabled", "budget_tokens": 4096, "display": "updates"}}"#,
        r#"{"thinking": {"type": "adaptive"}, "output_config": {"effort": "high"}}"#,
    ),
    (
        "claude-opus-4-7",
        r#"{"thinking": {"type": "enabled", "budget_tokens": 1000}, "output_config": {"effort": null, "format": "x"}}"#,
        r#"{"thinking": {"type": "adaptive"}, "output_config": {"effort": null, "format": "x"}}"#,
    ),
    (
        "claude-opus-4-7",
        r#"{"thinking": {"type": "enabled", "budget_tokens": 1000}, "output_config": "plain"}"#,
        r#"{"thinking": {"type": "adaptive"}, "output_config": {"effort": "low"}}"#,
    ),
    (
        "claude-opus-4-7",
        r#"{"thinking": {"type": "enabled"}}"#,
        r#"{"thinking": {"type": "adaptive"}, "output_config": {"effort": "low"}}"#,
    ),
    (
        "claude-opus-4-7",
        r#"{"thinking": {"type": "enabled", "budget_tokens": 5000.9}}"#,
        r#"{"thinking": {"type": "adaptive"}, "output_config": {"effort": "high"}}"#,
    ),
    (
        "claude-opus-4-6",
        r#"{"thinking": {"type": "enabled", "budget_tokens": 5000}}"#,
        r#"{"thinking": {"type": "enabled", "budget_tokens": 5000}}"#,
    ),
    (
        "claude-sonnet-4-5",
        r#"{"thinking": {"type": "enabled", "budget_tokens": 5000}}"#,
        r#"{"thinking": {"type": "enabled", "budget_tokens": 5000}}"#,
    ),
    (
        "claude-opus-4-7",
        r#"{"thinking": {"type": "disabled"}}"#,
        r#"{"thinking": {"type": "disabled"}}"#,
    ),
];

pub(super) async fn adaptive_thinking() {
    let fixture = Fixture::new(THINKING_MODELS, "{}");
    for (model, before, after) in TRANSLATIONS {
        let mut translated = params(before);
        translate(&fixture, model, &mut translated)
            .await
            .expect("translation");
        assert_eq!(translated, params(after), "{model} {before}");
    }
    let invalid_text = r#"{"thinking": {"type": "enabled", "budget_tokens": "many"}, "output_config": {"format": "x"}}"#;
    let mut invalid = params(invalid_text);
    assert!(matches!(
        translate(&fixture, "claude-opus-4-7", &mut invalid).await,
        Err(ThinkingError::BudgetTokens)
    ));
    assert_eq!(
        invalid,
        params(invalid_text),
        "the conversion fails before any mutation"
    );
}

const NO_DROP: DropParams = DropParams {
    global: false,
    call: false,
};
const GLOBAL_DROP: DropParams = DropParams {
    global: true,
    call: false,
};
const CALL_DROP: DropParams = DropParams {
    global: false,
    call: true,
};

fn assert_unsupported_params(class: Option<ErrorClass>, message: String) {
    let error_class = class.expect("an UnsupportedParamsError class");
    assert!(matches!(error_class, ErrorClass::UnsupportedParams(_)));
    let metadata = error_class.metadata();
    assert_eq!(metadata.status_code, 400);
    assert_eq!(metadata.error_type, WireField::MISSING);
    assert_eq!(metadata.param, WireField::Null);
    assert_eq!(metadata.code, "400");
    let facts = ErrorFacts {
        proxy_type: None,
        message,
        class: Class::from(&error_class),
        status_code: Some(400),
        rate_limit_type: None,
    };
    assert_eq!(
        Normalizer::new().unwrap().normalize(Some(&facts)),
        Some("400_INVALID_PARAMETER_VALUE")
    );
}

pub(super) fn forced_tool_downgrade() {
    let fixture = Fixture::new(FORCED_MODELS, "{}");
    let catalogs = fixture.catalogs();
    assert!(forced_tool_use_unsupported("claude-fable-5-1", &catalogs));
    assert!(!forced_tool_use_unsupported("claude-sonnet-4-5", &catalogs));
    let refused = forced_tool_use_downgraded("claude-fable-5-1", NO_DROP, &catalogs)
        .expect_err("no drop_params");
    assert_eq!(refused.status_code(), Some(400));
    assert_unsupported_params(refused.classify(), refused.to_string());
    assert!(ForcedToolChoiceError::MissingType.classify().is_none());
    assert_eq!(
        forced_tool_use_downgraded("claude-sonnet-4-5", NO_DROP, &catalogs),
        Ok(false)
    );
    let (guard, warnings) = capture_warnings();
    assert_eq!(
        forced_tool_use_downgraded("claude-fable-5-1", GLOBAL_DROP, &catalogs),
        Ok(true)
    );
    assert_eq!(
        warnings.messages(),
        [DROP_FORCED_TOOL_WARNING.replacen("%s", "claude-fable-5-1", 1)]
    );
    drop(guard);
    forced_tool_choices(&catalogs);
}

fn forced_tool_choices(catalogs: &CatalogSources<'_>) {
    let any = json(r#"{"type": "any"}"#);
    assert_eq!(
        apply_forced_tool_choice("claude-sonnet-4-5", &any, NO_DROP, catalogs),
        Ok(any.clone())
    );
    assert_eq!(
        apply_forced_tool_choice("claude-fable-5-1", &any, NO_DROP, catalogs),
        Err(ForcedToolChoiceError::Unsupported {
            model: "claude-fable-5-1".to_owned()
        })
    );
    let named = json(r#"{"type": "tool", "name": "lookup", "disable_parallel_tool_use": false}"#);
    let (guard, warnings) = capture_warnings();
    let downgraded = apply_forced_tool_choice("claude-fable-5-1", &named, CALL_DROP, catalogs);
    assert_eq!(
        downgraded,
        Ok(json(
            r#"{"type": "auto", "disable_parallel_tool_use": false}"#
        )),
        "the name is dropped and false parallel use is kept"
    );
    assert_eq!(warnings.messages().len(), 1);
    drop(guard);
    let null_parallel = json(r#"{"type": "any", "disable_parallel_tool_use": null}"#);
    assert_eq!(
        apply_forced_tool_choice("claude-fable-5-1", &null_parallel, GLOBAL_DROP, catalogs),
        Ok(json(r#"{"type": "auto"}"#))
    );
    other_tool_choices(catalogs);
}

fn other_tool_choices(catalogs: &CatalogSources<'_>) {
    let none = json(r#"{"type": "none"}"#);
    assert_eq!(
        apply_forced_tool_choice("claude-fable-5-1", &none, NO_DROP, catalogs),
        Ok(none.clone())
    );
    let nameless = json(r#"{"name": "lookup"}"#);
    assert_eq!(
        apply_forced_tool_choice("claude-fable-5-1", &nameless, NO_DROP, catalogs),
        Err(ForcedToolChoiceError::MissingType)
    );
    let text = json(r#""auto""#);
    assert_eq!(
        apply_forced_tool_choice("claude-fable-5-1", &text, NO_DROP, catalogs),
        Err(ForcedToolChoiceError::NotAnObject)
    );
}

pub(super) fn sampling_capability() {
    let fixture = Fixture::new(SAMPLING_MODELS, "{}");
    sampling_flags(&fixture);
    sampling_forwarding(&fixture);
    sampling_refusals(&fixture);
}

// The catalog flag wins over the name, so claude-fable-lite samples and claude-opus-4-7 does not.
const SAMPLING_FLAGS: [(&str, bool); 13] = [
    ("claude-sonnet-4-5", true),
    ("claude-opus-4-6", true),
    ("claude-fable-lite", true),
    ("claude-opus-4-7", false),
    ("claude-fable-5", false),
    ("CLAUDE-FABLE-5", false),
    ("claude-opus_4_7", false),
    ("claude-opus-4.7", false),
    ("claude-opus_4.7", false),
    ("claude-opus-4-8", false),
    ("claude-opus_4_8", false),
    ("claude-opus-4.8", false),
    ("claude-opus_4.8", false),
];

fn sampling_flags(fixture: &Fixture) {
    let catalogs = fixture.catalogs();
    for (model, supported) in SAMPLING_FLAGS {
        assert_eq!(
            supports_sampling_params(model, &catalogs),
            supported,
            "{model}"
        );
    }
}

fn sampling_forwarding(fixture: &Fixture) {
    let cases = [
        (
            "claude-sonnet-4-5",
            Temperature,
            "0.5",
            NO_DROP,
            Some("0.5"),
        ),
        ("claude-opus-4-8", Temperature, "1", NO_DROP, Some("1")),
        ("claude-opus-4-8", Temperature, "1.0", NO_DROP, Some("1.0")),
        (
            "claude-opus-4-8",
            Temperature,
            "true",
            NO_DROP,
            Some("true"),
        ),
        ("claude-opus-4-8", TopP, "0.5", GLOBAL_DROP, None),
        ("claude-opus-4-8", TopK, "5", CALL_DROP, None),
    ];
    for (model, param, value, drop_params, expected) in cases {
        let (guard, warnings) = capture_warnings();
        assert_eq!(
            sampling(fixture, model, param, &json(value), drop_params).expect("allowed"),
            expected.map(json),
            "{model} {param:?} {value}"
        );
        assert!(
            warnings.messages().is_empty(),
            "the sampling drop is silent: {model} {param:?} {value}"
        );
        drop(guard);
    }
}

fn sampling_refusals(fixture: &Fixture) {
    temperature_refusal(fixture);
    other_refusals(fixture);
}

fn temperature_refusal(fixture: &Fixture) {
    let half = json("0.5");
    let (guard, warnings) = capture_warnings();
    let temperature = sampling(fixture, "claude-opus-4-8", Temperature, &half, NO_DROP)
        .expect_err("no drop_params");
    assert_eq!(temperature.status_code(), Some(400));
    assert_unsupported_params(temperature.classify(), temperature.to_string());
    let message = temperature.to_string();
    assert!(message.contains("claude-opus-4-8 does not support temperature=0.5."));
    assert!(message.contains("Only temperature=1 is supported."));
    assert!(message.contains("set `litellm.drop_params = True`"));
    assert!(warnings.messages().is_empty(), "the drop is silent");
    drop(guard);
}

fn other_refusals(fixture: &Fixture) {
    let half = json("0.5");
    let top_p =
        sampling(fixture, "claude-opus-4-8", TopP, &half, NO_DROP).expect_err("no drop_params");
    assert_eq!(top_p.status_code(), Some(400));
    assert!(
        !top_p
            .to_string()
            .contains("Only temperature=1 is supported.")
    );
    let top_k = sampling(fixture, "claude-opus-4-8", TopK, &json("5"), NO_DROP)
        .expect_err("no drop_params");
    assert_eq!(top_k.status_code(), Some(400));
    assert!(top_k.to_string().contains("does not support top_k=5."));
    // LiteLLM forwards a value of 1 only for temperature
    for param in [TopP, TopK] {
        let refused = sampling(fixture, "claude-opus-4-8", param, &json("1"), NO_DROP)
            .expect_err("only temperature=1 is forwarded");
        assert_eq!(refused.status_code(), Some(400), "{param:?}=1");
    }
}

pub(super) async fn legacy_budget_effort() {
    let fixture = Fixture::new(EFFORT_MODELS, "{}");
    let defaults = EffortThresholds::default();
    let custom = EffortThresholds {
        medium: 10,
        high: 20,
        xhigh: 30,
    };
    // claude-sonnet-4-6 lacks supports_xhigh_reasoning_effort, so it never reaches xhigh or max
    let cases = [
        ("claude-opus-4-6", 8192, defaults, "xhigh"),
        ("claude-opus-4-6", 0x4000, defaults, "xhigh"),
        ("claude-sonnet-4-6", 0x4000, defaults, "high"),
        ("claude-opus-4-6", 8191, defaults, "high"),
        ("claude-opus-4-6", 4096, defaults, "high"),
        ("claude-opus-4-6", 4095, defaults, "medium"),
        ("claude-opus-4-6", 2048, defaults, "medium"),
        ("claude-opus-4-6", 2047, defaults, "low"),
        ("claude-opus-4-6", 0, defaults, "low"),
        ("claude-opus-4-6", -5, defaults, "low"),
        ("claude-opus-4-6", 30, custom, "xhigh"),
        ("claude-sonnet-4-6", 30, custom, "high"),
        ("claude-opus-4-6", 20, custom, "high"),
        ("claude-opus-4-6", 10, custom, "medium"),
        ("claude-opus-4-6", 9, custom, "low"),
    ];
    for (model, budget, thresholds, expected) in cases {
        assert_eq!(
            effort(&fixture, model, budget, thresholds).await,
            expected,
            "{model} with budget {budget}"
        );
    }
}

pub(super) async fn disabled_thinking_warning() {
    let fixture = Fixture::new(DISABLED_MODELS, "{}");
    let (guard, warnings) = capture_warnings();
    let mut disabled = params(r#"{"thinking": {"type": "disabled"}}"#);
    assert!(drop_disabled(&fixture, "claude-fable-5", &mut disabled).await);
    assert_eq!(disabled.thinking, None);
    assert_eq!(
        warnings.messages(),
        [DROP_DISABLED_THINKING_WARNING.replacen("%s", "claude-fable-5", 1)]
    );
    drop(guard);
    other_thinking_values(&fixture).await;
    let mut other_model = params(r#"{"thinking": {"type": "disabled"}}"#);
    assert!(!drop_disabled(&fixture, "claude-sonnet-4-5", &mut other_model).await);
    assert_eq!(other_model.thinking, Some(json(r#"{"type": "disabled"}"#)));
}

async fn other_thinking_values(fixture: &Fixture) {
    let mut enabled_params = params(r#"{"thinking": {"type": "enabled"}}"#);
    assert!(!drop_disabled(fixture, "claude-fable-5", &mut enabled_params).await);
    assert_eq!(
        enabled_params.thinking,
        Some(json(r#"{"type": "enabled"}"#))
    );
    let mut text_params = params(r#"{"thinking": "disabled"}"#);
    assert!(!drop_disabled(fixture, "claude-fable-5", &mut text_params).await);
    assert_eq!(text_params.thinking, Some(json(r#""disabled""#)));
    let mut absent = OptionalParams::default();
    assert!(!drop_disabled(fixture, "claude-fable-5", &mut absent).await);
    assert_eq!(absent.thinking, None);
}

const EFFORT_CASES: [(&str, &str, bool); 14] = [
    ("claude-sonnet-4-5", "{}", false),
    (
        "claude-opus-4-7",
        r#"{"output_config": {"effort": "high"}}"#,
        false,
    ),
    ("claude-opus-4-7", r#"{"reasoning_effort": "high"}"#, false),
    ("claude-opus-4-5", r#"{"reasoning_effort": "high"}"#, true),
    ("claude-opus_4_5", r#"{"reasoning_effort": "low"}"#, true),
    ("claude-opus-4-5", r#"{"reasoning_effort": ""}"#, false),
    ("claude-opus-4-5", r#"{"reasoning_effort": 5}"#, false),
    (
        "claude-sonnet-4-5",
        r#"{"reasoning_effort": "high"}"#,
        false,
    ),
    (
        "claude-sonnet-4-5",
        r#"{"output_config": {"effort": "medium"}}"#,
        true,
    ),
    (
        "claude-sonnet-4-5",
        r#"{"output_config": {"effort": ""}}"#,
        false,
    ),
    (
        "claude-sonnet-4-5",
        r#"{"output_config": {"effort": 5}}"#,
        false,
    ),
    ("claude-sonnet-4-5", r#"{"output_config": "plain"}"#, false),
    ("claude-sonnet-4-5", r#"{"output_config": {}}"#, false),
    ("", r#"{"output_config": {"effort": "high"}}"#, true),
];

pub(super) async fn effort_beta_gate() {
    let fixture = Fixture::new(ADAPTIVE_MODELS, "{}");
    assert!(!effort_used(&fixture, "claude-sonnet-4-5", None).await);
    for (model, text, expected) in EFFORT_CASES {
        let optional_params = params(text);
        assert_eq!(
            effort_used(&fixture, model, Some(&optional_params)).await,
            expected,
            "{model} {text}"
        );
    }
}
