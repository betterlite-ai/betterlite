use betterlite_models::decisions::Json;

use super::fixture::{Fixture, Session, capture_warnings};
use crate::anthropic_capability_catalog::capability::supports_model_capability;
use crate::anthropic_capability_catalog::forced_tool::{
    DROP_FORCED_TOOL_WARNING, ForcedToolChoiceError, apply_forced_tool_choice,
    forced_tool_use_downgraded, forced_tool_use_unsupported,
};
use crate::anthropic_capability_catalog::sampling::{
    SamplingError, SamplingParam, SamplingRequest, apply_sampling_param, supports_sampling_params,
};
use crate::anthropic_capability_catalog::thinking::{
    DROP_DISABLED_THINKING_WARNING, EffortRequest, ThinkingError, ThinkingRequest, is_effort_used,
    legacy_budget_to_effort, maybe_drop_disabled_thinking, translate_legacy_thinking,
};
use crate::anthropic_capability_catalog::{DropParams, EffortThresholds, OptionalParams};

const CATALOG_MODELS: &str = r#"{
    "anthropic/claude-x": {"litellm_provider": "anthropic", "supports_adaptive_thinking": true},
    "claude-y": {"litellm_provider": "anthropic", "supports_adaptive_thinking": false},
    "claude-z": {"litellm_provider": "anthropic"}
}"#;
const CATALOG_PACKAGED: &str = r#"{
    "claude-x": {"thinking_always_on": true},
    "claude-y": {"supports_adaptive_thinking": true},
    "claude-z": {"supports_legacy_thinking": true}
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
}

pub(super) async fn adaptive_thinking() {
    let fixture = Fixture::new(THINKING_MODELS, "{}");
    display_translation(&fixture).await;
    overlay_translation(&fixture).await;
    untouched_translation(&fixture).await;
    budget_edges(&fixture).await;
}

async fn display_translation(fixture: &Fixture) {
    let mut displayed = params(
        r#"{"thinking": {"type": "enabled", "budget_tokens": 5000, "display": "summarized"}}"#,
    );
    translate(fixture, "claude-opus-4-7", &mut displayed)
        .await
        .expect("translation");
    assert_eq!(
        displayed.thinking,
        Some(json(r#"{"type": "adaptive", "display": "summarized"}"#))
    );
    assert_eq!(displayed.output_config, Some(json(r#"{"effort": "high"}"#)));
    let mut updated =
        params(r#"{"thinking": {"type": "enabled", "budget_tokens": 4096, "display": "updates"}}"#);
    translate(fixture, "claude-opus-4-7", &mut updated)
        .await
        .expect("translation");
    assert_eq!(updated.thinking, Some(json(r#"{"type": "adaptive"}"#)));
}

async fn overlay_translation(fixture: &Fixture) {
    let mut overlaid = params(
        r#"{"thinking": {"type": "enabled", "budget_tokens": 1000}, "output_config": {"effort": null, "format": "x"}}"#,
    );
    translate(fixture, "claude-opus-4-7", &mut overlaid)
        .await
        .expect("translation");
    assert_eq!(
        overlaid.output_config,
        Some(json(r#"{"effort": null, "format": "x"}"#)),
        "the caller's effort wins and its other keys survive"
    );
    let mut discarded = params(
        r#"{"thinking": {"type": "enabled", "budget_tokens": 1000}, "output_config": "plain"}"#,
    );
    translate(fixture, "claude-opus-4-7", &mut discarded)
        .await
        .expect("translation");
    assert_eq!(
        discarded.output_config,
        Some(json(r#"{"effort": "low"}"#)),
        "a non-dict output_config is discarded"
    );
}

async fn untouched_translation(fixture: &Fixture) {
    let mut legacy = params(r#"{"thinking": {"type": "enabled", "budget_tokens": 5000}}"#);
    translate(fixture, "claude-opus-4-6", &mut legacy)
        .await
        .expect("legacy model");
    assert_eq!(
        legacy.thinking,
        Some(json(r#"{"type": "enabled", "budget_tokens": 5000}"#))
    );
    assert_eq!(legacy.output_config, None);
    let mut plain = params(r#"{"thinking": {"type": "enabled", "budget_tokens": 5000}}"#);
    translate(fixture, "claude-sonnet-4-5", &mut plain)
        .await
        .expect("non-adaptive model");
    assert_eq!(
        plain.thinking,
        Some(json(r#"{"type": "enabled", "budget_tokens": 5000}"#))
    );
    let mut disabled = params(r#"{"thinking": {"type": "disabled"}}"#);
    translate(fixture, "claude-opus-4-7", &mut disabled)
        .await
        .expect("disabled thinking");
    assert_eq!(disabled.thinking, Some(json(r#"{"type": "disabled"}"#)));
}

async fn budget_edges(fixture: &Fixture) {
    let mut invalid = params(
        r#"{"thinking": {"type": "enabled", "budget_tokens": "many"}, "output_config": {"format": "x"}}"#,
    );
    assert!(matches!(
        translate(fixture, "claude-opus-4-7", &mut invalid).await,
        Err(ThinkingError::BudgetTokens)
    ));
    assert_eq!(
        invalid.thinking,
        Some(json(r#"{"type": "enabled", "budget_tokens": "many"}"#))
    );
    assert_eq!(
        invalid.output_config,
        Some(json(r#"{"format": "x"}"#)),
        "the conversion fails before any mutation"
    );
    let mut absent = params(r#"{"thinking": {"type": "enabled"}}"#);
    translate(fixture, "claude-opus-4-7", &mut absent)
        .await
        .expect("absent budget");
    assert_eq!(absent.output_config, Some(json(r#"{"effort": "low"}"#)));
    let mut float = params(r#"{"thinking": {"type": "enabled", "budget_tokens": 5000.9}}"#);
    translate(fixture, "claude-opus-4-7", &mut float)
        .await
        .expect("float budget");
    assert_eq!(float.output_config, Some(json(r#"{"effort": "high"}"#)));
}

pub(super) fn forced_tool_downgrade() {
    let fixture = Fixture::new(FORCED_MODELS, "{}");
    let catalogs = fixture.catalogs();
    assert!(forced_tool_use_unsupported("claude-fable-5-1", &catalogs));
    assert!(!forced_tool_use_unsupported("claude-sonnet-4-5", &catalogs));
    let refused = forced_tool_use_downgraded("claude-fable-5-1", DropParams::default(), &catalogs)
        .expect_err("no drop_params");
    assert_eq!(refused.status_code(), Some(400));
    assert_eq!(
        forced_tool_use_downgraded("claude-sonnet-4-5", DropParams::default(), &catalogs),
        Ok(false)
    );
    let (guard, warnings) = capture_warnings();
    assert_eq!(
        forced_tool_use_downgraded(
            "claude-fable-5-1",
            DropParams {
                global: true,
                call: false
            },
            &catalogs
        ),
        Ok(true)
    );
    assert_eq!(
        warnings.messages(),
        [DROP_FORCED_TOOL_WARNING.replacen("%s", "claude-fable-5-1", 1)]
    );
    drop(guard);
    let any = json(r#"{"type": "any"}"#);
    assert_eq!(
        apply_forced_tool_choice("claude-sonnet-4-5", &any, DropParams::default(), &catalogs),
        Ok(any.clone())
    );
    assert_eq!(
        apply_forced_tool_choice("claude-fable-5-1", &any, DropParams::default(), &catalogs),
        Err(ForcedToolChoiceError::Unsupported {
            model: "claude-fable-5-1".to_owned()
        })
    );
    let named = json(r#"{"type": "tool", "name": "lookup", "disable_parallel_tool_use": false}"#);
    let (guard, warnings) = capture_warnings();
    let downgraded = apply_forced_tool_choice(
        "claude-fable-5-1",
        &named,
        DropParams {
            global: false,
            call: true,
        },
        &catalogs,
    )
    .expect("downgrade");
    assert_eq!(
        downgraded,
        json(r#"{"type": "auto", "disable_parallel_tool_use": false}"#),
        "the name is dropped and false parallel use is kept"
    );
    assert_eq!(warnings.messages().len(), 1);
    drop(guard);
    let null_parallel = json(r#"{"type": "any", "disable_parallel_tool_use": null}"#);
    let drop = DropParams {
        global: true,
        call: false,
    };
    assert_eq!(
        apply_forced_tool_choice("claude-fable-5-1", &null_parallel, drop, &catalogs),
        Ok(json(r#"{"type": "auto"}"#))
    );
    let none = json(r#"{"type": "none"}"#);
    assert_eq!(
        apply_forced_tool_choice("claude-fable-5-1", &none, DropParams::default(), &catalogs),
        Ok(none.clone())
    );
    assert_eq!(
        apply_forced_tool_choice(
            "claude-fable-5-1",
            &json(r#"{"name": "lookup"}"#),
            DropParams::default(),
            &catalogs
        ),
        Err(ForcedToolChoiceError::MissingType)
    );
    assert_eq!(
        apply_forced_tool_choice(
            "claude-fable-5-1",
            &json(r#""auto""#),
            DropParams::default(),
            &catalogs
        ),
        Err(ForcedToolChoiceError::NotAnObject)
    );
}

pub(super) fn sampling_capability() {
    let fixture = Fixture::new(SAMPLING_MODELS, "{}");
    sampling_flags(&fixture);
    sampling_forwarding(&fixture);
    sampling_refusals(&fixture);
}

fn sampling_flags(fixture: &Fixture) {
    let catalogs = fixture.catalogs();
    assert!(supports_sampling_params("claude-sonnet-4-5", &catalogs));
    assert!(supports_sampling_params("claude-opus-4-6", &catalogs));
    assert!(!supports_sampling_params("claude-opus-4-7", &catalogs));
    assert!(supports_sampling_params("claude-fable-lite", &catalogs));
    assert!(!supports_sampling_params("claude-fable-5", &catalogs));
    assert!(!supports_sampling_params("CLAUDE-FABLE-5", &catalogs));
    assert!(!supports_sampling_params("claude-opus-4-8", &catalogs));
    assert!(!supports_sampling_params("claude-opus_4_8", &catalogs));
    assert!(!supports_sampling_params("claude-opus-4.8", &catalogs));
    assert!(!supports_sampling_params("claude-opus_4.8", &catalogs));
    assert!(!supports_sampling_params("claude-opus-4.7", &catalogs));
    assert!(!supports_sampling_params("claude-opus_4.7", &catalogs));
}

fn sampling_forwarding(fixture: &Fixture) {
    let half = json("0.5");
    assert_eq!(
        sampling(
            fixture,
            "claude-sonnet-4-5",
            SamplingParam::Temperature,
            &half,
            DropParams::default()
        )
        .expect("supported"),
        Some(half.clone())
    );
    assert_eq!(
        sampling(
            fixture,
            "claude-opus-4-8",
            SamplingParam::Temperature,
            &json("1"),
            DropParams::default()
        )
        .expect("temperature 1"),
        Some(json("1"))
    );
    assert_eq!(
        sampling(
            fixture,
            "claude-opus-4-8",
            SamplingParam::Temperature,
            &json("1.0"),
            DropParams::default()
        )
        .expect("float temperature 1"),
        Some(json("1.0"))
    );
    assert_eq!(
        sampling(
            fixture,
            "claude-opus-4-8",
            SamplingParam::Temperature,
            &json("true"),
            DropParams::default()
        )
        .expect("bool temperature 1"),
        Some(json("true"))
    );
    assert_eq!(
        sampling(
            fixture,
            "claude-opus-4-8",
            SamplingParam::TopP,
            &half,
            DropParams {
                global: true,
                call: false
            }
        )
        .expect("global drop"),
        None
    );
    assert_eq!(
        sampling(
            fixture,
            "claude-opus-4-8",
            SamplingParam::TopK,
            &json("5"),
            DropParams {
                global: false,
                call: true
            }
        )
        .expect("call drop"),
        None
    );
}

fn sampling_refusals(fixture: &Fixture) {
    temperature_refusal(fixture);
    other_refusals(fixture);
}

fn temperature_refusal(fixture: &Fixture) {
    let half = json("0.5");
    let (guard, warnings) = capture_warnings();
    let temperature = sampling(
        fixture,
        "claude-opus-4-8",
        SamplingParam::Temperature,
        &half,
        DropParams::default(),
    )
    .expect_err("no drop_params");
    assert_eq!(temperature.status_code(), Some(400));
    let message = temperature.to_string();
    assert!(message.contains("claude-opus-4-8 does not support temperature=0.5."));
    assert!(message.contains("Only temperature=1 is supported."));
    assert!(message.contains("set `litellm.drop_params = True`"));
    assert!(warnings.messages().is_empty(), "the drop is silent");
    drop(guard);
}

fn other_refusals(fixture: &Fixture) {
    let half = json("0.5");
    let top_p = sampling(
        fixture,
        "claude-opus-4-8",
        SamplingParam::TopP,
        &half,
        DropParams::default(),
    )
    .expect_err("no drop_params");
    assert_eq!(top_p.status_code(), Some(400));
    assert!(
        !top_p
            .to_string()
            .contains("Only temperature=1 is supported.")
    );
    let top_k = sampling(
        fixture,
        "claude-opus-4-8",
        SamplingParam::TopK,
        &json("5"),
        DropParams::default(),
    )
    .expect_err("no drop_params");
    assert_eq!(top_k.status_code(), Some(400));
    assert!(top_k.to_string().contains("does not support top_k=5."));
}

pub(super) async fn legacy_budget_effort() {
    let fixture = Fixture::new(EFFORT_MODELS, "{}");
    let defaults = EffortThresholds::default();
    assert_eq!(
        effort(&fixture, "claude-opus-4-6", 8192, defaults).await,
        "xhigh"
    );
    assert_eq!(
        effort(&fixture, "claude-opus-4-6", 0x4000, defaults).await,
        "xhigh"
    );
    assert_eq!(
        effort(&fixture, "claude-sonnet-4-6", 0x4000, defaults).await,
        "high",
        "there is no max result even past the max threshold"
    );
    assert_eq!(
        effort(&fixture, "claude-opus-4-6", 8191, defaults).await,
        "high"
    );
    assert_eq!(
        effort(&fixture, "claude-opus-4-6", 4096, defaults).await,
        "high"
    );
    let defaults = EffortThresholds::default();
    assert_eq!(
        effort(&fixture, "claude-opus-4-6", 4095, defaults).await,
        "medium"
    );
    assert_eq!(
        effort(&fixture, "claude-opus-4-6", 2048, defaults).await,
        "medium"
    );
    assert_eq!(
        effort(&fixture, "claude-opus-4-6", 2047, defaults).await,
        "low"
    );
    assert_eq!(
        effort(&fixture, "claude-opus-4-6", 0, defaults).await,
        "low"
    );
    assert_eq!(
        effort(&fixture, "claude-opus-4-6", -5, defaults).await,
        "low"
    );
    let custom = EffortThresholds {
        medium: 10,
        high: 20,
        xhigh: 30,
    };
    assert_eq!(
        effort(&fixture, "claude-opus-4-6", 30, custom).await,
        "xhigh"
    );
    assert_eq!(
        effort(&fixture, "claude-sonnet-4-6", 30, custom).await,
        "high",
        "xhigh needs the model's supports_xhigh_reasoning_effort"
    );
    assert_eq!(
        effort(&fixture, "claude-opus-4-6", 20, custom).await,
        "high"
    );
    assert_eq!(
        effort(&fixture, "claude-opus-4-6", 10, custom).await,
        "medium"
    );
    assert_eq!(effort(&fixture, "claude-opus-4-6", 9, custom).await, "low");
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
    let enabled = json(r#"{"thinking": {"type": "enabled"}}"#);
    let text = json(r#"{"thinking": "disabled"}"#);
    let mut enabled_params = params(r#"{"thinking": {"type": "enabled"}}"#);
    assert!(!drop_disabled(fixture, "claude-fable-5", &mut enabled_params).await);
    assert_eq!(enabled_params.thinking, Some(enabled));
    let mut text_params = params(r#"{"thinking": "disabled"}"#);
    assert!(!drop_disabled(fixture, "claude-fable-5", &mut text_params).await);
    assert_eq!(text_params.thinking, Some(text));
    let mut absent = OptionalParams::default();
    assert!(!drop_disabled(fixture, "claude-fable-5", &mut absent).await);
    assert_eq!(absent.thinking, None);
}

pub(super) async fn effort_beta_gate() {
    let fixture = Fixture::new(ADAPTIVE_MODELS, "{}");
    assert!(!effort_used(&fixture, "claude-sonnet-4-5", None).await);
    assert!(
        !effort_used(&fixture, "claude-sonnet-4-5", Some(&params("{}"))).await,
        "absent or empty options return false"
    );
    assert!(
        !effort_used(
            &fixture,
            "claude-opus-4-7",
            Some(&params(r#"{"output_config": {"effort": "high"}}"#))
        )
        .await,
        "an adaptive model suppresses the effort beta"
    );
    assert!(
        !effort_used(
            &fixture,
            "claude-opus-4-7",
            Some(&params(r#"{"reasoning_effort": "high"}"#))
        )
        .await
    );
    let high = params(r#"{"reasoning_effort": "high"}"#);
    assert!(effort_used(&fixture, "claude-opus-4-5", Some(&high)).await);
    let low = params(r#"{"reasoning_effort": "low"}"#);
    assert!(effort_used(&fixture, "claude-opus_4_5", Some(&low)).await);
    let empty = params(r#"{"reasoning_effort": ""}"#);
    assert!(!effort_used(&fixture, "claude-opus-4-5", Some(&empty)).await);
    let number = params(r#"{"reasoning_effort": 5}"#);
    assert!(!effort_used(&fixture, "claude-opus-4-5", Some(&number)).await);
    assert!(
        !effort_used(&fixture, "claude-sonnet-4-5", Some(&high)).await,
        "reasoning_effort needs the opus-4-5 name"
    );
    let medium = params(r#"{"output_config": {"effort": "medium"}}"#);
    assert!(effort_used(&fixture, "claude-sonnet-4-5", Some(&medium)).await);
    let empty_config = params(r#"{"output_config": {"effort": ""}}"#);
    assert!(!effort_used(&fixture, "claude-sonnet-4-5", Some(&empty_config)).await);
    let number_effort = params(r#"{"output_config": {"effort": 5}}"#);
    assert!(!effort_used(&fixture, "claude-sonnet-4-5", Some(&number_effort)).await);
    let text = params(r#"{"output_config": "plain"}"#);
    assert!(!effort_used(&fixture, "claude-sonnet-4-5", Some(&text)).await);
    let blank = params(r#"{"output_config": {}}"#);
    assert!(!effort_used(&fixture, "claude-sonnet-4-5", Some(&blank)).await);
}
