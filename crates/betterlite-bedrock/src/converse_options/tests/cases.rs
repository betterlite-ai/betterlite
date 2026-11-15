use betterlite_anthropic::capabilities::effort::EffortError;
use betterlite_config::sections::anthropic_capability_catalog::Settings;

use crate::converse_options::budget::clamp_thinking_budget_tokens;
use crate::converse_options::config::{class_attributes, config_blocks, get_config};
use crate::converse_options::effort::{nova_reasoning_config, output_config_effort};
use crate::converse_options::error::{
    BEDROCK_CONVERSE_PROVIDER, ConverseOptionsError, NOVA_2_REJECTION_MODEL,
};
use crate::converse_options::family::{
    is_deepseek_model, is_nova_2_model, is_openai_gpt_reasoning_model, rejects_reasoning_effort,
    requires_min_max_tokens,
};
use crate::converse_options::reasoning::{ReasoningSources, handle_reasoning_effort};
use crate::converse_options::supported::{
    accepts_thinking_param, auto_tool_choice, supported_openai_params, supports_sampling_params,
};
use crate::converse_options::web_search::map_web_search_options;
use crate::model_routes::{ParamMap, ParamValue};

use super::support::{Answers, Fixture, Session, debug_logged, params, runtime, value};

const CAPABILITY_MAP: &str = r#"{
    "anthropic.claude-sonnet-4-20250514-v1:0": {"litellm_provider": "bedrock", "supports_sampling_params": false, "supports_tool_choice": true},
    "amazon.nova-pro-v1:0": {"litellm_provider": "bedrock", "supports_sampling_params": true, "supports_tool_choice": true},
    "amazon.nova-2-lite-v1:0": {"litellm_provider": "bedrock", "supports_sampling_params": false},
    "deepseek.v3-v1:0": {"litellm_provider": "bedrock", "supports_reasoning": true, "supports_sampling_params": true},
    "us.deepseek.v3-v1:0": {"litellm_provider": "bedrock", "supports_sampling_params": true},
    "deepseek.r1-v1:0": {"litellm_provider": "bedrock", "supports_reasoning": true, "supports_sampling_params": true},
    "openai.gpt-oss-120b-1:0": {"litellm_provider": "bedrock", "supports_sampling_params": true},
    "us.openai.gpt-5": {"litellm_provider": "bedrock", "supports_sampling_params": true},
    "nova/openai.gpt-5": {"litellm_provider": "bedrock", "supports_reasoning": false, "supports_sampling_params": true, "supports_tool_choice": false},
    "amazon.nova-custom": {"litellm_provider": "bedrock", "supports_reasoning": false, "supports_sampling_params": true, "supports_tool_choice": false},
    "cohere.command-r-v1:0": {"litellm_provider": "bedrock", "supports_sampling_params": true},
    "fixture-region": {"litellm_provider": "bedrock"},
    "global.fixture-region": {"litellm_provider": "bedrock", "supports_sampling_params": false},
    "us.fixture-priority": {"litellm_provider": "bedrock", "supports_sampling_params": true},
    "global.fixture-priority": {"litellm_provider": "bedrock", "supports_sampling_params": false},
    "us.fixture-us-fallback": {"litellm_provider": "bedrock", "supports_sampling_params": false},
    "fixture-unknown": {"litellm_provider": "bedrock"},
    "fixture-reasoning": {"litellm_provider": "bedrock", "supports_reasoning": true, "supports_sampling_params": true},
    "fixture-plain": {"litellm_provider": "bedrock", "supports_reasoning": false, "supports_sampling_params": true},
    "fixture-tools": {"litellm_provider": "bedrock", "supports_function_calling": true, "supports_sampling_params": true},
    "fixture-tool-off": {"litellm_provider": "bedrock", "supports_function_calling": false, "supports_sampling_params": true},
    "fixture-tool-choice": {"litellm_provider": "bedrock", "supports_tool_choice": true, "supports_sampling_params": true}
}"#;

const ADAPTIVE_MAP: &str = r#"{
    "fixture-adaptive": {"litellm_provider": "bedrock", "supports_adaptive_thinking": true, "supports_sampling_params": true},
    "fixture-plain": {"litellm_provider": "bedrock", "supports_sampling_params": true},
    "fixture-xhigh": {"litellm_provider": "bedrock", "supports_adaptive_thinking": true, "supports_xhigh_reasoning_effort": true}
}"#;

const BASE_PARAMS: [&str; 12] = [
    "max_tokens",
    "max_completion_tokens",
    "stream",
    "stream_options",
    "stop",
    "temperature",
    "top_p",
    "extra_headers",
    "response_format",
    "requestMetadata",
    "service_tier",
    "parallel_tool_calls",
];

fn names(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

fn handle(
    fixture: &Fixture,
    session: &mut Session<'_>,
    model: &str,
    effort: &str,
    optional: &mut ParamMap,
) -> Result<(), ConverseOptionsError> {
    let catalogs = fixture.catalogs();
    let budgets = Settings::default();
    let lookup = Answers::new(&[]);
    let sources = ReasoningSources {
        catalogs: &catalogs,
        budgets: &budgets,
        lookup: &lookup,
        packaged: fixture.packaged(),
    };
    let mut environment = session.environment();
    runtime().block_on(handle_reasoning_effort(
        model,
        effort,
        optional,
        &mut environment,
        &sources,
    ))
}

fn supported(fixture: &Fixture, session: &mut Session<'_>, model: &str) -> Vec<String> {
    let catalogs = fixture.catalogs();
    let mut environment = session.environment();
    runtime().block_on(supported_openai_params(model, &mut environment, &catalogs))
}

fn accepts_thinking(session: &mut Session<'_>, model: &str, base_model: &str) -> bool {
    let mut environment = session.environment();
    runtime().block_on(accepts_thinking_param(model, base_model, &mut environment))
}

pub(super) fn nova2_reasoning_config() {
    for model in [
        "amazon.nova-2-lite-v1:0",
        "us.amazon.nova-2-pro-preview-20251202-v1:0",
        "eu.amazon.nova-2-lite-v1:0",
        "apac.amazon.nova-2-lite-v1:0",
        "bedrock/converse/us.amazon.nova-2-lite-v1:0",
        "bedrock/apac.amazon.nova-2-lite-v1:0",
        "converse/eu.amazon.nova-2-lite-v1:0",
        "bedrock/converse/nova-2/imported-v1:0",
        "nova-2/imported-v1:0",
    ] {
        assert!(is_nova_2_model(model), "{model} is a Nova 2 model");
    }
    for model in [
        "amazon.nova-pro-v1:0",
        "Amazon.Nova-2-lite-v1:0",
        "global.amazon.nova-2-lite-v1:0",
        "us.us.amazon.nova-2-lite-v1:0",
        "bedrock/converse/bedrock/amazon.nova-2-lite-v1:0",
        "nova-2imported-v1:0",
        "amazon.nova-2-lite-v1:0/us.",
    ] {
        assert!(!is_nova_2_model(model), "{model} is not a Nova 2 model");
    }
    for effort in ["low", "medium", "high"] {
        let config = nova_reasoning_config(effort).expect("a valid effort");
        let expected = params(&format!(
            r#"{{"reasoningConfig": {{"type": "enabled", "maxReasoningEffort": "{effort}"}}}}"#
        ));
        assert_eq!(config, expected, "{effort}");
    }
    for effort in ["minimal", "xhigh", "max", "none", "", "High", "high "] {
        let error = nova_reasoning_config(effort).expect_err("a rejected effort");
        assert_eq!(
            error.to_string(),
            format!(
                "Invalid reasoning_effort value '{effort}' for Nova 2 models. Supported values: ['low', 'medium', 'high']"
            )
        );
        assert_eq!(error.status_code(), Some(400));
        let ConverseOptionsError::NovaEffort {
            model, provider, ..
        } = &error
        else {
            panic!("a Nova rejection");
        };
        assert_eq!(model, NOVA_2_REJECTION_MODEL);
        assert_eq!(provider, BEDROCK_CONVERSE_PROVIDER);
    }
}

pub(super) fn gpt_reasoning_object() {
    for model in [
        "openai.gpt-5",
        "us.openai.gpt-5-2025-08-07",
        "bedrock/converse/openai.gpt-6",
        "prefixopenai.gpt-4suffix",
        "openai.gpt-\u{661}",
    ] {
        assert!(
            is_openai_gpt_reasoning_model(model),
            "{model} is a GPT reasoning model"
        );
    }
    for model in [
        "openai.gpt-",
        "openai.gpt-x",
        "openai.gptoss",
        "openai-gpt-5",
        "OpenAI.GPT-5",
        "openai.gpt",
    ] {
        assert!(
            !is_openai_gpt_reasoning_model(model),
            "{model} is not a GPT reasoning model"
        );
    }
    assert!(requires_min_max_tokens("xai.grok-4"));
    assert!(requires_min_max_tokens("us.openai.gpt-5"));
    assert!(!requires_min_max_tokens("xai-grok-4"));
    assert!(!requires_min_max_tokens("amazon.nova-2-lite-v1:0"));

    let fixture = Fixture::new("{}", "{}");
    let mut session = Session::new(&fixture);
    let mut optional = params(
        r#"{"reasoning": {"effort": "old", "budget_tokens": 2}, "thinking": {"type": "enabled"}}"#,
    );
    handle(
        &fixture,
        &mut session,
        "bedrock/converse/us.openai.gpt-5",
        "ultra",
        &mut optional,
    )
    .expect("the GPT branch stores the raw effort");
    assert_eq!(
        optional.get("reasoning"),
        Some(&value(r#"{"effort": "ultra"}"#)),
        "the old reasoning object is replaced without validating the effort"
    );
    assert_eq!(
        optional.get("thinking"),
        Some(&value(r#"{"type": "enabled"}"#)),
        "the GPT branch leaves unrelated keys alone"
    );
    assert_eq!(optional.get("reasoning_effort"), None);

    let mut mixed = ParamMap::new();
    handle(
        &fixture,
        &mut session,
        "gpt-oss/openai.gpt-5",
        "low",
        &mut mixed,
    )
    .expect("the gpt-oss prefix wins over the GPT detector");
    assert_eq!(
        mixed.get("reasoning_effort"),
        Some(&ParamValue::Text("low".to_owned()))
    );
    assert_eq!(mixed.get("reasoning"), None);

    let mut nova_gpt = ParamMap::new();
    handle(
        &fixture,
        &mut session,
        "nova-2/openai.gpt-5",
        "minimal",
        &mut nova_gpt,
    )
    .expect("the GPT detector wins over the Nova 2 prefix");
    assert_eq!(
        nova_gpt.get("reasoning"),
        Some(&value(r#"{"effort": "minimal"}"#))
    );
    assert_eq!(nova_gpt.get("reasoningConfig"), None);
}

pub(super) fn gpt_oss_and_deepseek_effort() {
    let fixture = Fixture::new(CAPABILITY_MAP, "{}");
    let mut session = Session::new(&fixture);
    for (model, effort) in [
        ("bedrock/converse/openai.gpt-oss-120b-1:0", "none"),
        ("bedrock/converse/deepseek.v3-v1:0", "unknown-effort"),
        ("bedrock/converse/deepseek.r1-v1:0", "high"),
    ] {
        let mut optional = params(r#"{"thinking": {"type": "enabled", "budget_tokens": 2048}}"#);
        handle(&fixture, &mut session, model, effort, &mut optional)
            .expect("the raw effort passes through");
        assert_eq!(
            optional.get("reasoning_effort"),
            Some(&ParamValue::Text(effort.to_owned())),
            "{model}"
        );
        assert_eq!(
            optional.get("thinking"),
            Some(&value(r#"{"type": "enabled", "budget_tokens": 2048}"#)),
            "{model} keeps the caller's thinking"
        );
    }
    assert!(is_deepseek_model("deepseek.v3-v1:0", ""));
    assert!(is_deepseek_model("", "us.deepseek.v3-v1:0"));
    assert!(!is_deepseek_model(
        "anthropic.claude-sonnet-4-20250514-v1:0",
        "amazon.nova-pro-v1:0"
    ));
    assert!(rejects_reasoning_effort("deepseek.r1-v1:0", ""));
    assert!(rejects_reasoning_effort("", "bedrock/deepseek.r1-v1:0"));
    assert!(!rejects_reasoning_effort(
        "deepseek.v3-v1:0",
        "deepseek.v3-v1:0"
    ));
    assert!(!rejects_reasoning_effort("amazon.nova-pro-v1:0", ""));

    let v3 = supported(&fixture, &mut session, "deepseek.v3-v1:0");
    assert!(
        v3.contains(&"reasoning_effort".to_owned()),
        "DeepSeek V3 lists reasoning_effort: {v3:?}"
    );
    assert!(
        !v3.contains(&"thinking".to_owned()),
        "DeepSeek V3 never lists the Anthropic thinking field: {v3:?}"
    );
    let region = supported(&fixture, &mut session, "us.deepseek.v3-v1:0");
    assert!(
        region.contains(&"reasoning_effort".to_owned()),
        "the base model keeps the DeepSeek family: {region:?}"
    );
    let r1 = supported(&fixture, &mut session, "deepseek.r1-v1:0");
    assert!(
        !r1.contains(&"reasoning_effort".to_owned()),
        "DeepSeek R1 rejects reasoning_effort: {r1:?}"
    );
    assert!(
        !accepts_thinking(&mut session, "deepseek.v3-v1:0", "deepseek.v3-v1:0"),
        "a supports_reasoning flag never makes DeepSeek accept thinking"
    );
}

pub(super) fn anthropic_adaptive_effort() {
    for (effort, mapped) in [
        ("low", "low"),
        ("minimal", "low"),
        ("medium", "medium"),
        ("high", "high"),
        ("xhigh", "xhigh"),
        ("max", "max"),
    ] {
        assert_eq!(output_config_effort(effort), Some(mapped), "{effort}");
    }
    for effort in ["none", "", "High", "high "] {
        assert_eq!(output_config_effort(effort), None, "{effort}");
    }

    let fixture = Fixture::new(ADAPTIVE_MAP, "{}");
    let mut session = Session::new(&fixture);

    let mut adaptive = params(
        r#"{"thinking": {"type": "enabled", "budget_tokens": 1024}, "output_config": {"format": "x"}}"#,
    );
    handle(
        &fixture,
        &mut session,
        "fixture-adaptive",
        "medium",
        &mut adaptive,
    )
    .expect("an adaptive effort maps");
    assert_eq!(
        adaptive.get("thinking"),
        Some(&value(r#"{"type": "adaptive", "display": "summarized"}"#))
    );
    assert_eq!(
        adaptive.get("output_config"),
        Some(&value(r#"{"format": "x", "effort": "medium"}"#)),
        "an existing output_config is kept around the added effort"
    );
    assert_eq!(
        adaptive.get("_output_config_normalized"),
        Some(&ParamValue::Bool(true))
    );

    let mut minimal = ParamMap::new();
    handle(
        &fixture,
        &mut session,
        "fixture-adaptive",
        "minimal",
        &mut minimal,
    )
    .expect("minimal folds into low");
    assert_eq!(
        minimal.get("output_config"),
        Some(&value(r#"{"effort": "low"}"#))
    );

    let mut cleared = params(
        r#"{"thinking": {"type": "enabled", "budget_tokens": 2048}, "output_config": {"effort": "high"}, "_output_config_normalized": false}"#,
    );
    handle(
        &fixture,
        &mut session,
        "fixture-adaptive",
        "none",
        &mut cleared,
    )
    .expect("none removes the reasoning fields");
    assert_eq!(cleared.get("thinking"), None);
    assert_eq!(cleared.get("output_config"), None);
    assert_eq!(
        cleared.get("_output_config_normalized"),
        Some(&ParamValue::Bool(false)),
        "the none branch leaves an existing marker alone"
    );

    let mut existing = params(r#"{"output_config": {"effort": "ultra", "format": "x"}}"#);
    let error = handle(
        &fixture,
        &mut session,
        "fixture-adaptive",
        "low",
        &mut existing,
    )
    .expect_err("the existing effort is validated");
    assert_eq!(error.status_code(), Some(400));
    assert_eq!(
        error.to_string(),
        "Invalid reasoning_effort/output_config.effort value: 'ultra'. Must be one of: 'low', 'medium', 'high', 'xhigh', or 'max'."
    );
    let ConverseOptionsError::AdaptiveEffort {
        model, provider, ..
    } = &error
    else {
        panic!("an invalid adaptive effort");
    };
    assert_eq!(model, "fixture-adaptive");
    assert_eq!(provider, BEDROCK_CONVERSE_PROVIDER);

    let mut invalid = ParamMap::new();
    let error = handle(
        &fixture,
        &mut session,
        "fixture-adaptive",
        "ultra",
        &mut invalid,
    )
    .expect_err("an unmappable effort is refused");
    assert_eq!(error.status_code(), Some(400));
    let ConverseOptionsError::InvalidEffort(EffortError::InvalidReasoningEffort {
        model,
        provider,
        ..
    }) = &error
    else {
        panic!("the mapping refuses the effort");
    };
    assert_eq!(model, "fixture-adaptive");
    assert_eq!(provider, BEDROCK_CONVERSE_PROVIDER);
    assert!(matches!(error, ConverseOptionsError::InvalidEffort(_)));

    let mut xhigh = ParamMap::new();
    let error = handle(
        &fixture,
        &mut session,
        "fixture-adaptive",
        "xhigh",
        &mut xhigh,
    )
    .expect_err("xhigh needs the model capability");
    assert_eq!(error.status_code(), Some(400));
    assert!(matches!(
        &error,
        ConverseOptionsError::UnsupportedEffort { .. }
    ));
    let mut allowed = ParamMap::new();
    handle(
        &fixture,
        &mut session,
        "fixture-xhigh",
        "xhigh",
        &mut allowed,
    )
    .expect("a model with the xhigh effort accepts it");
    assert_eq!(
        allowed.get("output_config"),
        Some(&value(r#"{"effort": "xhigh"}"#))
    );

    let mut plain = ParamMap::new();
    handle(
        &fixture,
        &mut session,
        "fixture-plain",
        "medium",
        &mut plain,
    )
    .expect("a non-adaptive effort enables thinking");
    assert_eq!(
        plain.get("thinking"),
        Some(&value(r#"{"type": "enabled", "budget_tokens": 2048}"#))
    );
    assert_eq!(plain.get("output_config"), None);
    assert_eq!(plain.get("_output_config_normalized"), None);

    let mut unhashable = params(r#"{"output_config": {"effort": ["high"]}}"#);
    let error = handle(
        &fixture,
        &mut session,
        "fixture-adaptive",
        "low",
        &mut unhashable,
    )
    .expect_err("an unhashable effort mirrors the Python TypeError");
    assert_eq!(error.status_code(), None);
    assert!(matches!(error, ConverseOptionsError::Capability(_)));
}

pub(super) fn minimum_thinking_budget() {
    fn clamp(source: &str, minimum: i64) -> ParamMap {
        let mut optional = params(source);
        clamp_thinking_budget_tokens(&mut optional, minimum);
        optional
    }
    assert_eq!(
        clamp(
            r#"{"thinking": {"type": "enabled", "budget_tokens": 512}}"#,
            1024
        ),
        params(r#"{"thinking": {"type": "enabled", "budget_tokens": 1024}}"#)
    );
    assert_eq!(
        clamp(r#"{"thinking": {"budget_tokens": 512}}"#, 1024),
        params(r#"{"thinking": {"budget_tokens": 1024}}"#),
        "no thinking type is required"
    );
    assert_eq!(
        clamp(r#"{"thinking": {"budget_tokens": 1023}}"#, 1024),
        params(r#"{"thinking": {"budget_tokens": 1024}}"#),
        "one below the minimum clamps"
    );
    assert_eq!(
        clamp(r#"{"thinking": {"budget_tokens": 1024}}"#, 1024),
        params(r#"{"thinking": {"budget_tokens": 1024}}"#),
        "the minimum itself is kept"
    );
    assert_eq!(
        clamp(
            r#"{"thinking": {"budget_tokens": 16384}, "max_tokens": 1024}"#,
            1024
        ),
        params(r#"{"thinking": {"budget_tokens": 16384}, "max_tokens": 1024}"#),
        "no upper clamp applies"
    );
    assert_eq!(
        clamp(r#"{"thinking": {"budget_tokens": true}}"#, 1024),
        params(r#"{"thinking": {"budget_tokens": 1024}}"#),
        "a boolean counts as an integer"
    );
    assert_eq!(
        clamp(r#"{"thinking": {"budget_tokens": false}}"#, 1024),
        params(r#"{"thinking": {"budget_tokens": 1024}}"#)
    );
    assert_eq!(
        clamp(r#"{"thinking": {"budget_tokens": 512}}"#, 2048),
        params(r#"{"thinking": {"budget_tokens": 2048}}"#),
        "the configured minimum is used"
    );
    for source in [
        r#"{"thinking": {"budget_tokens": 512.0}}"#,
        r#"{"thinking": {"budget_tokens": "512"}}"#,
        r#"{"thinking": {"budget_tokens": null}}"#,
        r#"{"thinking": {"type": "enabled"}}"#,
        r#"{"thinking": "enabled"}"#,
        r#"{}"#,
    ] {
        assert_eq!(clamp(source, 1024), params(source), "{source} is untouched");
    }
    let (_, messages) = debug_logged(|| clamp(r#"{"thinking": {"budget_tokens": 512}}"#, 1024));
    assert_eq!(
        messages,
        vec![
            "Bedrock requires thinking.budget_tokens >= 1024, got 512. Clamping to minimum."
                .to_owned()
        ]
    );
    let (_, quiet) = debug_logged(|| clamp(r#"{"thinking": {"budget_tokens": 1024}}"#, 1024));
    assert!(quiet.is_empty());
}

pub(super) fn nova_web_search_only() {
    for model in [
        "amazon.nova-pro-v1:0",
        "us.amazon.nova-lite-v1:0",
        "NOVA-PRO",
        "my-nova-model",
    ] {
        let tool = map_web_search_options(model).expect("Nova enables grounding");
        assert_eq!(
            tool,
            params(r#"{"systemTool": {"name": "nova_grounding"}}"#),
            "{model}"
        );
    }
    assert_eq!(
        map_web_search_options("anthropic.claude-sonnet-4-20250514-v1:0"),
        None
    );
    assert_eq!(map_web_search_options("amazon.titan-text-v2"), None);
    assert_eq!(map_web_search_options(""), None);
    let (_, messages) = debug_logged(|| map_web_search_options("amazon.titan-text-v2"));
    assert_eq!(
        messages,
        vec![
            "web_search_options passed but model amazon.titan-text-v2 is not a Nova model. Nova grounding is only supported on Amazon Nova models."
                .to_owned()
        ]
    );
}

pub(super) fn sampling_capabilities() {
    let fixture = Fixture::new(CAPABILITY_MAP, "{}");
    let catalogs = fixture.catalogs();
    assert!(
        supports_sampling_params("anthropic.claude-sonnet-4-20250514-v1:0", &catalogs),
        "an anthropic base wins over an explicit false"
    );
    assert!(supports_sampling_params(
        "us.anthropic.claude-sonnet-4-20250514-v1:0",
        &catalogs
    ));
    assert!(!supports_sampling_params(
        "amazon.nova-2-lite-v1:0",
        &catalogs
    ));
    assert!(supports_sampling_params("amazon.nova-pro-v1:0", &catalogs));
    assert!(
        supports_sampling_params("us.fixture-priority", &catalogs),
        "the us. candidate answers before the global. one"
    );
    assert!(!supports_sampling_params("fixture-region", &catalogs));
    assert!(
        !supports_sampling_params("us.fixture-us-fallback", &catalogs),
        "the us. candidate is queried"
    );
    assert!(
        supports_sampling_params("fixture-unknown", &catalogs),
        "an unmapped model supports sampling"
    );

    let mut session = Session::new(&fixture);
    for model in [
        "anthropic.claude-3-7-sonnet-20250219-v1:0",
        "us.anthropic.claude-sonnet-4-20250514-v1:0",
        "anthropic.claude-opus-4-20250514-v1:0",
    ] {
        assert!(
            accepts_thinking(&mut session, model, model),
            "{model} has a literal thinking shortcut"
        );
    }
    assert!(accepts_thinking(
        &mut session,
        "fixture-reasoning",
        "fixture-reasoning"
    ));
    assert!(
        accepts_thinking(&mut session, "fixture-plain", "fixture-reasoning"),
        "the base model's reasoning flag counts"
    );
    assert!(!accepts_thinking(
        &mut session,
        "fixture-plain",
        "fixture-plain"
    ));
    assert!(!accepts_thinking(
        &mut session,
        "us.deepseek.v3-v1:0",
        "deepseek.v3-v1:0"
    ));
}

pub(super) fn supported_param_list() {
    let fixture = Fixture::new(CAPABILITY_MAP, "{}");
    let mut session = Session::new(&fixture);

    let mut anthropic = names(&BASE_PARAMS);
    anthropic.extend(names(&[
        "tools",
        "tool_choice",
        "thinking",
        "reasoning_effort",
        "output_config",
        "context_management",
    ]));
    assert_eq!(
        supported(
            &fixture,
            &mut session,
            "anthropic.claude-sonnet-4-20250514-v1:0"
        ),
        anthropic
    );

    let mut nova = names(&BASE_PARAMS);
    nova.extend(names(&["tools", "web_search_options", "tool_choice"]));
    assert_eq!(
        supported(&fixture, &mut session, "amazon.nova-pro-v1:0"),
        nova
    );

    let mut nova2 = names(&BASE_PARAMS);
    nova2.extend(names(&["tools", "web_search_options"]));
    nova2.push("reasoning_effort".to_owned());
    assert_eq!(
        supported(&fixture, &mut session, "amazon.nova-2-lite-v1:0"),
        nova2
    );

    let mut deepseek = names(&BASE_PARAMS);
    deepseek.push("reasoning_effort".to_owned());
    assert_eq!(
        supported(&fixture, &mut session, "deepseek.v3-v1:0"),
        deepseek
    );
    assert_eq!(
        supported(&fixture, &mut session, "deepseek.r1-v1:0"),
        names(&BASE_PARAMS),
        "DeepSeek R1 lists no reasoning_effort"
    );

    let mut gpt_oss = names(&BASE_PARAMS);
    gpt_oss.push("reasoning_effort".to_owned());
    assert_eq!(
        supported(&fixture, &mut session, "openai.gpt-oss-120b-1:0"),
        gpt_oss
    );
    assert_eq!(
        supported(&fixture, &mut session, "us.openai.gpt-5"),
        gpt_oss
    );

    let mut wrapped_gpt = names(&BASE_PARAMS);
    wrapped_gpt.extend(names(&["tools", "web_search_options", "reasoning_effort"]));
    assert_eq!(
        supported(&fixture, &mut session, "nova/openai.gpt-5"),
        wrapped_gpt,
        "the nova wrapper hides the GPT marker from the base model alone"
    );

    let mut arn = names(&BASE_PARAMS);
    arn.extend(names(&[
        "tools",
        "tool_choice",
        "thinking",
        "reasoning_effort",
        "output_config",
        "web_search_options",
    ]));
    assert_eq!(
        supported(
            &fixture,
            &mut session,
            "arn:aws:bedrock:us-east-1:123456789012:inference-profile/nova-pro"
        ),
        arn
    );
    let mut plain_arn = names(&BASE_PARAMS);
    plain_arn.extend(names(&[
        "tools",
        "tool_choice",
        "thinking",
        "reasoning_effort",
        "output_config",
    ]));
    assert_eq!(
        supported(
            &fixture,
            &mut session,
            "arn:aws:bedrock:us-east-1:123456789012:inference-profile/claude"
        ),
        plain_arn
    );

    let mut cohere = names(&BASE_PARAMS);
    cohere.push("tools".to_owned());
    assert_eq!(
        supported(&fixture, &mut session, "cohere.command-r-v1:0"),
        cohere
    );

    let mut flagged = names(&BASE_PARAMS);
    flagged.push("tools".to_owned());
    assert_eq!(supported(&fixture, &mut session, "fixture-tools"), flagged);
    assert_eq!(
        supported(&fixture, &mut session, "fixture-tool-off"),
        names(&BASE_PARAMS),
        "an explicit false function-calling flag adds no tools"
    );
    let mut choice = names(&BASE_PARAMS);
    choice.push("tool_choice".to_owned());
    assert_eq!(
        supported(&fixture, &mut session, "fixture-tool-choice"),
        choice
    );

    assert_eq!(
        auto_tool_choice(),
        params(r#"{"auto": {}}"#),
        "auto_tool_choice wraps an empty auto block"
    );
}

pub(super) fn converse_class_defaults() {
    let empty = class_attributes(&ParamMap::new());
    assert_eq!(
        get_config(&empty),
        params(r#"{"custom_llm_provider": {"kind": "property"}}"#),
        "the annotated fields create no runtime attribute"
    );
    assert_eq!(
        config_blocks(),
        params(
            r#"{"guardrailConfig": "GuardrailConfigBlock", "performanceConfig": "PerformanceConfigBlock", "serviceTier": "ServiceTierBlock"}"#
        )
    );
    assert_eq!(
        config_blocks()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["guardrailConfig", "performanceConfig", "serviceTier"],
        "the block order is the class body's order"
    );

    let arguments = params(
        r#"{"maxTokens": 0, "stopSequences": [], "temperature": 0, "topP": null, "topK": 0}"#,
    );
    let attributes = class_attributes(&arguments);
    assert_eq!(
        get_config(&attributes),
        params(
            r#"{"custom_llm_provider": {"kind": "property"}, "maxTokens": 0, "stopSequences": [], "temperature": 0, "topK": 0}"#
        ),
        "falsey values are stored and the null argument is skipped"
    );

    let mut seeded = class_attributes(&params(r#"{"maxTokens": 16}"#));
    let extra = params(
        r#"{"_private": 1, "maxTokens": null, "get_config": {"kind": "classmethod"}, "configure": {"kind": "function"}, "help": {"kind": "builtin_function"}, "utility": {"kind": "staticmethod"}, "custom_llm_provider": {"kind": "property"}, "nested": {"kind": "other", "value": 2}}"#,
    );
    seeded.extend(extra);
    assert_eq!(
        get_config(&seeded),
        params(
            r#"{"custom_llm_provider": {"kind": "property"}, "nested": {"kind": "other", "value": 2}}"#
        ),
        "underscore names, nulls and callables are dropped"
    );
}
