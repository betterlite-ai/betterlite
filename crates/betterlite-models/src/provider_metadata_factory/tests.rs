#![cfg(test)]

use indexmap::IndexMap;
use rstest::rstest;
use strum::VariantArray as _;

use crate::decisions::Json;
use crate::entry::ModelFields;
use crate::fireworks_catalog_metadata::Catalog;
use crate::identity::provider_id::ProviderId;

use super::adapters::{
    AzureFoundryModelInfo, ContextLimitBehavior, DynamicModelInfo, FireworksSettings,
    LemonadeSettings, ProviderSpecificModelInfo, Stop, ToolChoice,
};
use super::selection::{MetadataQuery, ProviderModelMetadata, get_provider_info, select};

#[rstest]
#[case::metadata_adapter_selection(metadata_adapter_selection)]
#[case::provider_info_projection(provider_info_projection)]
#[case::metadata_constructors(metadata_constructors)]
#[case::dynamic_metadata_separation(dynamic_metadata_separation)]
fn provider_metadata_factory_cases(#[case] scenario: fn()) {
    scenario();
}

const SELECTIONS: &[(ProviderId, &str)] = &[
    (ProviderId::FireworksAi, "FireworksAIConfig"),
    (ProviderId::Openai, "OpenAIGPTConfig"),
    (ProviderId::Gemini, "GeminiModelInfo"),
    (ProviderId::VertexAi, "VertexAIModelInfo"),
    (ProviderId::LitellmProxy, "LiteLLMProxyChatConfig"),
    (ProviderId::Topaz, "TopazModelInfo"),
    (ProviderId::Anthropic, "AnthropicModelInfo"),
    (ProviderId::Xai, "XAIModelInfo"),
    (ProviderId::Ollama, "OllamaModelInfo"),
    (ProviderId::OllamaChat, "OllamaModelInfo"),
    (ProviderId::Vllm, "VLLMModelInfo"),
    (ProviderId::HostedVllm, "VLLMModelInfo"),
    (ProviderId::Lemonade, "LemonadeChatConfig"),
    (ProviderId::Clarifai, "ClarifaiConfig"),
    (ProviderId::Edenai, "EdenAIChatConfig"),
    (ProviderId::Bedrock, "BedrockModelInfo"),
    (ProviderId::AzureAi, "AzureFoundryModelInfo"),
];

const XAI_INFO: ProviderSpecificModelInfo = ProviderSpecificModelInfo {
    supports_function_calling: None,
    supports_prompt_caching: None,
    supports_reasoning: None,
    supports_vision: None,
    supports_pdf_input: None,
    supports_web_search: Some(true),
};

fn catalog_of(entries: &[(&str, &str)]) -> Catalog {
    entries
        .iter()
        .map(|(key, source)| {
            let fields: ModelFields = serde_json::from_str(source).unwrap();
            ((*key).to_owned(), fields)
        })
        .collect()
}

fn json(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn query(catalog: &Catalog) -> MetadataQuery<'_> {
    MetadataQuery {
        catalog,
        generation: 0,
    }
}

fn empty_fireworks() -> FireworksSettings {
    FireworksSettings::default()
}

fn empty_lemonade() -> LemonadeSettings {
    LemonadeSettings::default()
}

fn metadata_adapter_selection() {
    selected_classes();
    assert_eq!(
        select(Some("openai/gpt-4o-mini"), ProviderId::Openai)
            .unwrap()
            .class_name(),
        "OpenAIGPTConfig"
    );
    assert_eq!(
        select(None, ProviderId::Gemini).unwrap().class_name(),
        "GeminiModelInfo"
    );
    assert_eq!(
        foundry_model(Some("FW-GLM-5.2")).as_deref(),
        Some("FW-GLM-5.2")
    );
    assert_eq!(foundry_model(None), None);
    assert_eq!(foundry_model(Some("")).as_deref(), Some(""));
}

fn selected_classes() {
    for (provider, class) in SELECTIONS {
        assert_eq!(
            select(Some("model"), *provider).unwrap().class_name(),
            *class
        );
    }
    let known: Vec<ProviderId> = SELECTIONS.iter().map(|(provider, _)| *provider).collect();
    let unexpected: Vec<ProviderId> = ProviderId::VARIANTS
        .iter()
        .copied()
        .filter(|provider| !known.contains(provider))
        .filter(|provider| select(Some("model"), *provider).is_some())
        .collect();
    assert!(unexpected.is_empty(), "unexpected adapters: {unexpected:?}");
}

fn foundry_model(model: Option<&str>) -> Option<String> {
    match select(model, ProviderId::AzureAi) {
        Some(ProviderModelMetadata::AzureAi(config)) => config.model,
        other => panic!("expected the Foundry adapter, got {other:?}"),
    }
}

fn provider_info_projection() {
    inherited_projection();
    xai_projection();
    fireworks_projection();
    free_projection();
}

fn inherited_projection() {
    for (provider, _) in SELECTIONS {
        if matches!(provider, ProviderId::FireworksAi | ProviderId::Xai) {
            continue;
        }
        let mut selected = select(Some("model"), *provider).unwrap();
        assert_eq!(
            selected.get_provider_info("model", query(&catalog_of(&[]))),
            None,
            "{provider:?} inherits the base get_provider_info"
        );
        assert_eq!(
            selected.get_model_cost_key("model"),
            None,
            "{provider:?} inherits the base get_model_cost_key"
        );
    }
}

fn xai_projection() {
    let mut xai = select(Some("grok-4"), ProviderId::Xai).unwrap();
    assert_eq!(
        xai.get_provider_info("grok-4", query(&catalog_of(&[]))),
        Some(XAI_INFO)
    );
    assert_eq!(
        serde_json::to_string(
            &xai.get_provider_info("grok-4", query(&catalog_of(&[])))
                .unwrap()
        )
        .unwrap(),
        r#"{"supports_web_search":true}"#
    );
    assert_eq!(xai.get_model_cost_key("grok-4"), None);
}

fn fireworks_projection() {
    fireworks_cost_keys();
    let entries = catalog_of(&[
        (
            "fireworks_ai/glm-4p6",
            r#"{"supports_function_calling":false,"supports_vision":true}"#,
        ),
        (
            "fireworks_ai/accounts/fireworks/models/reasoning-on",
            r#"{"supports_reasoning":true}"#,
        ),
    ]);
    let mut fireworks = select(Some("glm-4p6"), ProviderId::FireworksAi).unwrap();
    assert_eq!(
        fireworks.get_provider_info("glm-4p6", query(&entries)),
        Some(ProviderSpecificModelInfo {
            supports_function_calling: Some(false),
            supports_prompt_caching: Some(true),
            supports_reasoning: None,
            supports_vision: Some(true),
            supports_pdf_input: None,
            supports_web_search: None,
        })
    );
    assert_eq!(
        fireworks.get_provider_info("unknown-model", query(&entries)),
        Some(ProviderSpecificModelInfo {
            supports_function_calling: Some(true),
            supports_prompt_caching: Some(true),
            supports_reasoning: None,
            supports_vision: None,
            supports_pdf_input: None,
            supports_web_search: None,
        })
    );
}

fn fireworks_cost_keys() {
    let fireworks = select(Some("glm-4p6"), ProviderId::FireworksAi).unwrap();
    assert_eq!(
        fireworks.get_model_cost_key("glm-4p6").as_deref(),
        Some("fireworks_ai/accounts/fireworks/models/glm-4p6")
    );
    assert_eq!(
        fireworks.get_model_cost_key("deepseek-r1").as_deref(),
        Some("fireworks_ai/accounts/fireworks/models/deepseek-r1")
    );
    assert_eq!(
        fireworks.get_model_cost_key("").as_deref(),
        Some("fireworks_ai/accounts/fireworks/models/")
    );
}

fn free_projection() {
    let free = catalog_of(&[(
        "fireworks_ai/glm-4p6",
        r#"{"supports_function_calling":false}"#,
    )]);
    assert_eq!(
        get_provider_info("grok-4", Some("xai"), query(&free)),
        Some(XAI_INFO)
    );
    // The guard is case-sensitive: only the exact slug selects the adapter.
    assert_eq!(get_provider_info("grok-4", Some("XAI"), query(&free)), None);
    assert_eq!(
        get_provider_info("glm-4p6", Some("fireworks_ai"), query(&free))
            .unwrap()
            .supports_function_calling,
        Some(false)
    );
    assert_eq!(
        get_provider_info("gpt-4o", Some("openai"), query(&free)),
        None
    );
    assert_eq!(get_provider_info("grok-4", None, query(&free)), None);
    assert_eq!(get_provider_info("grok-4", Some(""), query(&free)), None);
    assert_eq!(
        get_provider_info("grok-4", Some("not-a-provider"), query(&free)),
        None
    );
    assert_eq!(
        get_provider_info("grok-4", Some("OpenAI"), query(&free)),
        None
    );
    assert_eq!(
        get_provider_info("grok-4", Some("xai/"), query(&free)),
        None
    );
    assert_eq!(get_provider_info("grok-4", Some("gmi"), query(&free)), None);
    assert_eq!(
        get_provider_info("grok-4", Some("text-completion-openai"), query(&free)),
        None
    );
}

fn metadata_constructors() {
    fireworks_defaults();
    fireworks_constructor();
    fireworks_keeps_configured_values();
    lemonade_defaults();
    lemonade_constructor();
    foundry_defaults();
}

fn fireworks_defaults() {
    let defaults = FireworksSettings::default();
    assert_eq!(serde_json::to_string(&defaults).unwrap(), "{}");
    assert_eq!(defaults.clone().with_args(empty_fireworks()), defaults);
}

fn fireworks_constructor() {
    let args: FireworksSettings = serde_json::from_str(concat!(
        r#"{"tools":[{"type":"function"}],"tool_choice":"auto","max_tokens":7,"temperature":1,"#,
        r#""top_p":2,"top_k":3,"frequency_penalty":4,"presence_penalty":5,"n":6,"stop":["END"],"#,
        r#""response_format":{"type":"json_object"},"user":"user-1","logprobs":8,"#,
        r#""reasoning_effort":"high","prompt_truncate_len":9,"#,
        r#""context_length_exceeded_behavior":"truncate"}"#
    ))
    .unwrap();
    let constructed = FireworksSettings::default().with_args(args);
    assert_eq!(
        json(&serde_json::to_string(&constructed).unwrap()),
        json(concat!(
            r#"{"frequency_penalty":4,"max_tokens":7,"n":6,"stop":["END"],"temperature":1,"#,
            r#""top_p":2,"response_format":{"type":"json_object"},"#,
            r#""tools":[{"type":"function"}],"tool_choice":"auto","top_k":3,"#,
            r#""presence_penalty":5,"user":"user-1","logprobs":8,"reasoning_effort":"high","#,
            r#""prompt_truncate_len":9,"context_length_exceeded_behavior":"truncate"}"#
        ))
    );
}

fn fireworks_keeps_configured_values() {
    let base = FireworksSettings {
        top_k: Some(3),
        user: Some("kept".to_owned()),
        ..empty_fireworks()
    };
    assert_eq!(base.clone().with_args(empty_fireworks()), base);
    let merged = base.with_args(FireworksSettings {
        max_tokens: Some(0),
        temperature: Some(0),
        stop: Some(Stop::Text(String::new())),
        response_format: Some(IndexMap::new()),
        number: Some(0),
        user: Some(String::new()),
        tools: Some(Vec::new()),
        tool_choice: Some(ToolChoice::Name(String::new())),
        ..empty_fireworks()
    });
    assert_eq!(merged.temperature, Some(0));
    assert_eq!(merged.top_k, Some(3));
    assert_eq!(merged.user.as_deref(), Some(""));
    assert_eq!(merged.stop, Some(Stop::Text(String::new())));
    assert_eq!(merged.tools, Some(Vec::new()));
    assert_eq!(merged.response_format, Some(IndexMap::new()));
    assert_eq!(merged.number, Some(0));
    assert_eq!(merged.tool_choice, Some(ToolChoice::Name(String::new())));
    assert_eq!(
        json(&serde_json::to_string(&merged).unwrap()),
        json(concat!(
            r#"{"max_tokens":0,"n":0,"stop":"","temperature":0,"response_format":{},"#,
            r#""tools":[],"tool_choice":"","top_k":3,"user":""}"#
        ))
    );
}

fn lemonade_defaults() {
    let defaults = LemonadeSettings::default();
    assert_eq!(serde_json::to_string(&defaults).unwrap(), "{}");
    assert_eq!(defaults.clone().with_args(empty_lemonade()), defaults);
}

fn lemonade_constructor() {
    let constructed = LemonadeSettings::default().with_args(LemonadeSettings {
        repeat_penalty: Some(0.5_f64),
        functions: Some(vec![json(r#"{"name":"f"}"#)]),
        logit_bias: Some(IndexMap::from([(
            "50256".to_owned(),
            Json::Number(serde_json::Number::from(-100_i64)),
        )])),
        max_completion_tokens: Some(11),
        max_tokens: Some(12),
        number: Some(2),
        presence_penalty: Some(1),
        stop: Some(Stop::Text("END".to_owned())),
        temperature: Some(1),
        top_p: Some(2),
        top_k: Some(3),
        response_format: Some(IndexMap::from([(
            "type".to_owned(),
            Json::Text("text".to_owned()),
        )])),
        tools: Some(vec![json("true")]),
    });
    assert_eq!(
        json(&serde_json::to_string(&constructed).unwrap()),
        json(concat!(
            r#"{"repeat_penalty":0.5,"functions":[{"name":"f"}],"logit_bias":{"50256":-100},"#,
            r#""max_completion_tokens":11,"max_tokens":12,"n":2,"presence_penalty":1,"stop":"END","#,
            r#""temperature":1,"top_p":2,"top_k":3,"response_format":{"type":"text"},"tools":[true]}"#
        ))
    );
    let kept = constructed.with_args(LemonadeSettings {
        repeat_penalty: Some(0.0_f64),
        max_tokens: Some(0),
        functions: Some(Vec::new()),
        ..empty_lemonade()
    });
    assert_eq!(kept.repeat_penalty, Some(0.0_f64));
    assert_eq!(kept.max_tokens, Some(0));
    assert_eq!(kept.max_completion_tokens, Some(11));
    assert_eq!(kept.functions, Some(Vec::new()));
    assert_eq!(kept.tools, Some(vec![json("true")]));
}

fn foundry_defaults() {
    let adapter = AzureFoundryModelInfo { model: None };
    assert_eq!(adapter.model, None);
    assert_eq!(foundry_model(None), None);
}

fn dynamic_metadata_separation() {
    dynamic_markers();
    vllm_has_no_dynamic_query();
}

fn dynamic_markers() {
    for provider in [ProviderId::Ollama, ProviderId::OllamaChat] {
        assert_eq!(
            select(Some("qwen2.5"), provider)
                .unwrap()
                .dynamic_model_info(),
            Some(DynamicModelInfo::Ollama)
        );
    }
    assert_eq!(
        select(Some("qwen2.5"), ProviderId::Lemonade)
            .unwrap()
            .dynamic_model_info(),
        Some(DynamicModelInfo::Lemonade)
    );
    let others: Vec<ProviderId> = SELECTIONS
        .iter()
        .map(|(provider, _)| *provider)
        .filter(|provider| {
            !matches!(
                provider,
                ProviderId::Ollama | ProviderId::OllamaChat | ProviderId::Lemonade
            )
        })
        .collect();
    for provider in others {
        assert_eq!(
            select(Some("model"), provider)
                .unwrap()
                .dynamic_model_info(),
            None,
            "{provider:?} has no get_model_info"
        );
    }
}

fn vllm_has_no_dynamic_query() {
    assert_eq!(
        select(Some("model"), ProviderId::Vllm)
            .unwrap()
            .dynamic_model_info(),
        None
    );
    assert_eq!(
        select(Some("model"), ProviderId::HostedVllm)
            .unwrap()
            .dynamic_model_info(),
        None
    );
    let selected = select(Some("qwen2.5"), ProviderId::Ollama).unwrap();
    assert_eq!(selected.class_name(), "OllamaModelInfo");
    assert_eq!(
        selected.dynamic_model_info(),
        Some(DynamicModelInfo::Ollama)
    );
}

#[test]
fn fireworks_wire_names_parse_and_print() {
    let settings: FireworksSettings =
        serde_json::from_str(r#"{"stop":"END","tool_choice":{"type":"function"}}"#).unwrap();
    assert_eq!(settings.stop, Some(Stop::Text("END".to_owned())));
    assert_eq!(
        settings.tool_choice,
        Some(ToolChoice::Object(IndexMap::from([(
            "type".to_owned(),
            Json::Text("function".to_owned()),
        )])))
    );
    assert_eq!(
        json(&serde_json::to_string(&settings).unwrap()),
        json(r#"{"stop":"END","tool_choice":{"type":"function"}}"#)
    );
    let listed: FireworksSettings = serde_json::from_str(r#"{"stop":["a","b"],"n":3}"#).unwrap();
    assert_eq!(
        listed.stop,
        Some(Stop::List(vec!["a".to_owned(), "b".to_owned()]))
    );
    assert_eq!(listed.number, Some(3));
    assert_eq!(
        json(&serde_json::to_string(&listed).unwrap()),
        json(r#"{"n":3,"stop":["a","b"]}"#)
    );
}

#[test]
fn settings_reject_values_outside_the_declared_shape() {
    let error: FireworksSettings =
        serde_json::from_str(r#"{"context_length_exceeded_behavior":"error"}"#).unwrap();
    assert_eq!(
        error.context_limit_behavior,
        Some(ContextLimitBehavior::Error)
    );
    serde_json::from_str::<FireworksSettings>(r#"{"context_length_exceeded_behavior":"drop"}"#)
        .unwrap_err();
    serde_json::from_str::<FireworksSettings>(r#"{"unknown_key":1}"#).unwrap_err();
    serde_json::from_str::<LemonadeSettings>(r#"{"unknown_key":1}"#).unwrap_err();
}
