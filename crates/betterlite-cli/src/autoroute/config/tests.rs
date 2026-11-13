#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::AUTOROUTER_MODEL_NAME;
use super::classifier::{
    ClassifierChoice, ClassifierType, DEFAULT_CLASSIFIER_TIMEOUT_MS, LlmClassifierConfig,
};
use super::document::{
    ComplexityRouterConfig, DeploymentParams, GeneralSettings, GeneratedConfigDocument,
    ModelDeployment, build_generated_model_list, master_key_from_config,
};
use super::error::ConfigError;
use super::model::{
    DiscoveredModel, RawModelListing, chat_models, embedding_models, parse_discovered_models,
};
use super::semantic::{
    DEFAULT_KEYWORD_TIER_RULES, DEFAULT_MATCH_THRESHOLD, KeywordTierRule, SemanticMatching,
    SemanticMatchingChoice, default_keywords,
};
use super::settings::{AutorouteConfig, validate_config};
use super::tier::Tier;

fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

fn model(name: &str, mode: &str) -> DiscoveredModel {
    DiscoveredModel {
        name: name.to_owned(),
        mode: mode.to_owned(),
    }
}

fn names(models: &[DiscoveredModel]) -> Vec<&str> {
    models.iter().map(|model| model.name.as_str()).collect()
}

fn tier_map(entries: &[(Tier, &[&str])]) -> BTreeMap<Tier, Vec<String>> {
    entries
        .iter()
        .map(|(tier, models)| {
            (
                *tier,
                models.iter().map(|name| (*name).to_owned()).collect(),
            )
        })
        .collect()
}

fn discovered() -> Vec<DiscoveredModel> {
    vec![
        model("alpha", "chat"),
        model("embed", "embedding"),
        model("beta", "chat"),
    ]
}

fn all_tiers() -> BTreeMap<Tier, Vec<String>> {
    let mut tiers = BTreeMap::new();
    tiers.insert(Tier::Simple, vec!["alpha".to_owned()]);
    tiers.insert(Tier::Medium, vec!["alpha".to_owned(), "beta".to_owned()]);
    tiers.insert(Tier::Complex, vec!["beta".to_owned()]);
    tiers.insert(Tier::Reasoning, vec!["beta".to_owned()]);
    tiers
}

fn config() -> AutorouteConfig {
    AutorouteConfig {
        base_url: "https://proxy.example".to_owned(),
        api_key: secret("sk-test"),
        tiers: all_tiers(),
        default_model: "alpha".to_owned(),
        classifier: ClassifierChoice::Heuristic,
        semantic_matching: SemanticMatchingChoice::Disabled,
        adaptive: false,
    }
}

fn default_rules() -> Vec<KeywordTierRule> {
    Tier::ALL
        .iter()
        .map(|tier| KeywordTierRule {
            keywords: default_keywords(*tier)
                .iter()
                .map(|keyword| (*keyword).to_owned())
                .collect(),
            tier: *tier,
        })
        .collect()
}

fn classifier_and_semantic_config() -> AutorouteConfig {
    let mut config = config();
    config.classifier = ClassifierChoice::Llm(LlmClassifierConfig {
        model: "classifier".to_owned(),
        timeout_ms: DEFAULT_CLASSIFIER_TIMEOUT_MS,
    });
    config.semantic_matching = SemanticMatchingChoice::Enabled(SemanticMatching {
        embedding_model: "embed".to_owned(),
        match_threshold: DEFAULT_MATCH_THRESHOLD,
        keyword_tier_rules: default_rules(),
    });
    config
}

fn deployment_names(deployments: &[ModelDeployment]) -> Vec<&str> {
    deployments
        .iter()
        .map(|deployment| deployment.model_name.as_str())
        .collect()
}

fn router_config(config: &AutorouteConfig) -> ComplexityRouterConfig {
    let deployments = build_generated_model_list(config);
    let deployment = deployments
        .iter()
        .find(|deployment| deployment.model_name == AUTOROUTER_MODEL_NAME)
        .expect("the autorouter deployment");
    let DeploymentParams::AutoRouter(params) = &deployment.litellm_params else {
        panic!("the autorouter deployment carries the auto router params");
    };
    params.complexity_router_config.clone()
}

fn render(config: &AutorouteConfig) -> String {
    let document = GeneratedConfigDocument {
        model_list: build_generated_model_list(config),
        general_settings: None,
    };
    serde_saphyr::to_string(&document).expect("the generated document")
}

#[test]
fn config_types_preserve_every_source_field_and_iterate_tiers_in_declaration_order() {
    let discovered = model("alpha", "chat");
    assert_eq!(discovered.name, "alpha");
    assert_eq!(discovered.mode, "chat");

    let mut tiers = BTreeMap::new();
    tiers.insert(Tier::Reasoning, vec!["REASONING-model".to_owned()]);
    tiers.insert(Tier::Complex, vec!["COMPLEX-model".to_owned()]);
    tiers.insert(Tier::Medium, vec!["MEDIUM-model".to_owned()]);
    tiers.insert(Tier::Simple, vec!["SIMPLE-model".to_owned()]);
    let config = AutorouteConfig {
        base_url: "https://proxy.example".to_owned(),
        api_key: secret("sk-test"),
        tiers,
        default_model: "SIMPLE-model".to_owned(),
        classifier: ClassifierChoice::Heuristic,
        semantic_matching: SemanticMatchingChoice::Disabled,
        adaptive: true,
    };
    assert_eq!(config.base_url, "https://proxy.example");
    assert_eq!(config.api_key.expose_secret(), "sk-test");
    assert_eq!(config.default_model, "SIMPLE-model");
    assert!(config.adaptive);
    let order: Vec<Tier> = config.tiers.keys().copied().collect();
    assert_eq!(
        order,
        vec![Tier::Simple, Tier::Medium, Tier::Complex, Tier::Reasoning]
    );
    assert_eq!(
        Tier::ALL,
        [Tier::Simple, Tier::Medium, Tier::Complex, Tier::Reasoning]
    );
}

#[rstest]
#[case::simple(Tier::Simple, "SIMPLE")]
#[case::medium(Tier::Medium, "MEDIUM")]
#[case::complex(Tier::Complex, "COMPLEX")]
#[case::reasoning(Tier::Reasoning, "REASONING")]
fn tier_names_and_serialization_are_uppercase(#[case] tier: Tier, #[case] name: &str) {
    assert_eq!(tier.to_string(), name);
    let rendered = serde_saphyr::to_string(&tier).expect("a serialized tier");
    assert_eq!(rendered.trim(), name);
}

#[test]
fn llm_classifier_config_defaults_a_missing_timeout_to_three_seconds() {
    let config: LlmClassifierConfig =
        serde_saphyr::from_str("model: classifier\n").expect("a classifier config");
    assert_eq!(config.timeout_ms, 3000);
}

#[test]
fn parse_discovered_models_sets_the_name_from_the_id() {
    let raw = vec![RawModelListing {
        id: "gpt-4o".to_owned(),
        mode: "responses".to_owned(),
    }];
    let parsed = parse_discovered_models(&raw);
    assert_eq!(parsed, vec![model("gpt-4o", "responses")]);
}

#[test]
fn parse_discovered_models_defaults_a_missing_mode_to_chat() {
    let raw: Vec<RawModelListing> =
        serde_json::from_str(r#"[{"id":"claude"},{"id":"embed","mode":"embedding"}]"#)
            .expect("model listings");
    let parsed = parse_discovered_models(&raw);
    assert_eq!(
        parsed,
        vec![model("claude", "chat"), model("embed", "embedding")]
    );
}

#[test]
fn parse_discovered_models_ignores_extra_fields_and_requires_an_id() {
    let raw: Vec<RawModelListing> = serde_json::from_str(
        r#"[{"id":"claude","mode":"chat","max_input_tokens":200000,"unknown":true}]"#,
    )
    .expect("model listings");
    assert_eq!(parse_discovered_models(&raw), vec![model("claude", "chat")]);
    let _missing_id = serde_json::from_str::<Vec<RawModelListing>>(r#"[{"mode":"chat"}]"#)
        .expect_err("id is required");
}

#[test]
fn chat_models_keeps_only_chat_mode_in_discovery_order() {
    let models = vec![
        model("alpha", "chat"),
        model("embed", "embedding"),
        model("responses", "responses"),
        model("beta", "chat"),
    ];
    assert_eq!(names(&chat_models(&models)), vec!["alpha", "beta"]);
}

#[test]
fn embedding_models_keeps_only_embedding_mode() {
    let models = vec![
        model("alpha", "chat"),
        model("embed", "embedding"),
        model("other", "embedding"),
    ];
    assert_eq!(names(&embedding_models(&models)), vec!["embed", "other"]);
}

#[test]
fn chat_models_and_embedding_models_return_owned_values() {
    let kept = {
        let models = vec![model("alpha", "chat"), model("embed", "embedding")];
        (chat_models(&models), embedding_models(&models))
    };
    assert_eq!(names(&kept.0), vec!["alpha"]);
    assert_eq!(names(&kept.1), vec!["embed"]);
    assert!(chat_models(&[model("embed", "embedding")]).is_empty());
    assert!(embedding_models(&[model("alpha", "chat")]).is_empty());
}

#[test]
fn default_keyword_tier_rules_hold_one_entry_per_tier_in_declaration_order() {
    let tiers: Vec<Tier> = DEFAULT_KEYWORD_TIER_RULES
        .iter()
        .map(|(_keywords, tier)| *tier)
        .collect();
    assert_eq!(tiers, Tier::ALL.to_vec());
    assert_eq!(
        default_keywords(Tier::Simple),
        ["hi", "hello", "thanks"].as_slice()
    );
    assert_eq!(
        default_keywords(Tier::Medium),
        ["explain", "how does"].as_slice()
    );
    assert_eq!(
        default_keywords(Tier::Complex),
        ["refactor", "implement", "debug"].as_slice()
    );
    assert_eq!(
        default_keywords(Tier::Reasoning),
        ["step by step", "think through", "prove"].as_slice()
    );
}

#[test]
fn validate_config_accepts_a_config_referencing_only_discovered_models() {
    assert_eq!(validate_config(&config(), &discovered()), Ok(()));
}

#[test]
fn validate_config_rejects_a_tier_model_outside_the_chat_pool() {
    let mut config = config();
    config.tiers.insert(Tier::Simple, vec!["embed".to_owned()]);
    let error = validate_config(&config, &discovered()).expect_err("a non-chat tier model");
    let message = error.to_string();
    assert!(matches!(
        error,
        ConfigError::TierModelNotDiscovered {
            tier: Tier::Simple,
            ..
        }
    ));
    assert_eq!(message, "Tier SIMPLE references unknown chat model 'embed'");
}

#[test]
fn validate_config_rejects_an_unknown_default_model_against_the_chat_pool() {
    let mut config = config();
    config.default_model = "embed".to_owned();
    let error = validate_config(&config, &discovered()).expect_err("an embedding default model");
    assert!(matches!(
        error,
        ConfigError::DefaultModelNotDiscovered { .. }
    ));
    assert_eq!(
        error.to_string(),
        "default_model 'embed' is not a known chat model"
    );
}

#[test]
fn validate_config_checks_the_llm_classifier_model_against_the_chat_pool() {
    let mut config = config();
    config.classifier = ClassifierChoice::Llm(LlmClassifierConfig {
        model: "embed".to_owned(),
        timeout_ms: DEFAULT_CLASSIFIER_TIMEOUT_MS,
    });
    let error = validate_config(&config, &discovered()).expect_err("an embedding classifier");
    assert!(matches!(
        error,
        ConfigError::ClassifierModelNotDiscovered { .. }
    ));
    assert_eq!(
        error.to_string(),
        "classifier model 'embed' is not a known chat model"
    );
}

#[test]
fn validate_config_skips_the_classifier_check_for_the_heuristic_scorer() {
    let mut config = config();
    config.classifier = ClassifierChoice::Heuristic;
    assert_eq!(validate_config(&config, &discovered()), Ok(()));
}

#[test]
fn validate_config_checks_the_embedding_model_against_the_embedding_pool() {
    let mut config = config();
    config.semantic_matching = SemanticMatchingChoice::Enabled(SemanticMatching {
        embedding_model: "alpha".to_owned(),
        match_threshold: DEFAULT_MATCH_THRESHOLD,
        keyword_tier_rules: default_rules(),
    });
    let error = validate_config(&config, &discovered()).expect_err("a chat model as the embedder");
    assert!(matches!(
        error,
        ConfigError::EmbeddingModelNotDiscovered { .. }
    ));
    assert_eq!(
        error.to_string(),
        "embedding model 'alpha' is not a known embedding model"
    );
}

#[test]
fn validate_config_skips_the_embedding_check_when_semantic_matching_is_disabled() {
    let mut config = config();
    config.semantic_matching = SemanticMatchingChoice::Disabled;
    let without_embeddings = vec![model("alpha", "chat"), model("beta", "chat")];
    assert_eq!(validate_config(&config, &without_embeddings), Ok(()));
}

#[test]
fn validate_config_reports_the_first_failing_field_in_litellm_order() {
    let mut config = config();
    config.tiers.insert(Tier::Simple, vec!["embed".to_owned()]);
    config.default_model = "embed".to_owned();
    config.classifier = ClassifierChoice::Llm(LlmClassifierConfig {
        model: "embed".to_owned(),
        timeout_ms: DEFAULT_CLASSIFIER_TIMEOUT_MS,
    });
    config.semantic_matching = SemanticMatchingChoice::Enabled(SemanticMatching {
        embedding_model: "alpha".to_owned(),
        match_threshold: DEFAULT_MATCH_THRESHOLD,
        keyword_tier_rules: default_rules(),
    });
    let error = validate_config(&config, &discovered()).expect_err("every field is wrong");
    assert!(matches!(error, ConfigError::TierModelNotDiscovered { .. }));
}

#[test]
fn build_generated_model_list_deduplicates_and_sorts_referenced_models() {
    let mut config = config();
    config.tiers = tier_map(&[
        (Tier::Simple, &["zeta", "alpha"]),
        (Tier::Medium, &["alpha"]),
        (Tier::Complex, &["beta"]),
        (Tier::Reasoning, &["beta"]),
    ]);
    config.default_model = "alpha".to_owned();
    let deployments = build_generated_model_list(&config);
    assert_eq!(
        deployment_names(&deployments),
        vec!["alpha", "beta", "zeta", AUTOROUTER_MODEL_NAME]
    );
}

#[test]
fn build_generated_model_list_forwards_every_referenced_model_to_the_proxy() {
    let config = classifier_and_semantic_config();
    let deployments = build_generated_model_list(&config);
    assert_eq!(
        deployment_names(&deployments),
        vec![
            "alpha",
            "beta",
            "classifier",
            "embed",
            AUTOROUTER_MODEL_NAME
        ]
    );
    let forwarded: Vec<(&str, &str, &str)> = deployments
        .iter()
        .filter_map(|deployment| match &deployment.litellm_params {
            DeploymentParams::Proxy(params) => Some((
                params.model.as_str(),
                params.api_base.as_str(),
                params.api_key.expose_secret(),
            )),
            DeploymentParams::AutoRouter(_params) => None,
        })
        .collect();
    assert_eq!(
        forwarded,
        vec![
            ("litellm_proxy/alpha", "https://proxy.example", "sk-test"),
            ("litellm_proxy/beta", "https://proxy.example", "sk-test"),
            (
                "litellm_proxy/classifier",
                "https://proxy.example",
                "sk-test"
            ),
            ("litellm_proxy/embed", "https://proxy.example", "sk-test"),
        ]
    );
}

#[test]
fn build_generated_model_list_appends_exactly_one_autorouter_deployment() {
    let deployments = build_generated_model_list(&config());
    let router_names: Vec<&str> = deployments
        .iter()
        .filter(|deployment| deployment.model_name == AUTOROUTER_MODEL_NAME)
        .map(|deployment| deployment.model_name.as_str())
        .collect();
    assert_eq!(router_names, vec!["autorouter"]);
    let router = deployments.last().expect("the autorouter deployment");
    let DeploymentParams::AutoRouter(params) = &router.litellm_params else {
        panic!("the last deployment is the complex router");
    };
    assert_eq!(params.model, "auto_router/complexity_router");
    assert_eq!(params.complexity_router_config.default_model, "alpha");
}

#[test]
fn build_generated_model_list_serializes_the_complexity_router_config() {
    let mut config = classifier_and_semantic_config();
    config.semantic_matching = SemanticMatchingChoice::Enabled(SemanticMatching {
        embedding_model: "embed".to_owned(),
        match_threshold: 0.75,
        keyword_tier_rules: default_rules(),
    });
    config.adaptive = true;
    let router = router_config(&config);
    assert_eq!(router.tiers, all_tiers());
    assert_eq!(router.default_model, "alpha");
    assert!(router.return_raw_model_name);
    assert_eq!(router.classifier_type, Some(ClassifierType::Llm));
    assert_eq!(
        router.classifier_llm_config,
        Some(LlmClassifierConfig {
            model: "classifier".to_owned(),
            timeout_ms: 3000,
        })
    );
    assert!(router.semantic.semantic_keyword_matching);
    assert_eq!(router.semantic.embedding_model.as_deref(), Some("embed"));
    assert_eq!(router.semantic.match_threshold, Some(0.75_f64));
    assert_eq!(router.semantic.keyword_tier_rules, Some(default_rules()));
    assert!(router.adaptive);
}

#[test]
fn build_generated_model_list_omits_optional_router_keys_when_they_are_disabled() {
    let rendered = render(&config());
    assert!(!rendered.contains("classifier_type"));
    assert!(!rendered.contains("classifier_llm_config"));
    assert!(!rendered.contains("semantic_keyword_matching"));
    assert!(!rendered.contains("embedding_model"));
    assert!(!rendered.contains("match_threshold"));
    assert!(!rendered.contains("keyword_tier_rules"));
    assert!(!rendered.contains("adaptive"));
    assert!(rendered.contains("return_raw_model_name: true"));
    assert!(rendered.contains("SIMPLE:"));
    assert!(rendered.contains("MEDIUM:"));
    assert!(rendered.contains("COMPLEX:"));
    assert!(rendered.contains("REASONING:"));
}

#[test]
fn build_generated_model_list_is_stable_across_runs() {
    let config = config();
    assert_eq!(render(&config), render(&config));
}

#[test]
fn master_key_from_config_returns_the_key_verbatim() {
    let document = GeneratedConfigDocument {
        model_list: Vec::new(),
        general_settings: Some(GeneralSettings {
            master_key: Some(secret(" sk-untouched ")),
        }),
    };
    let key = master_key_from_config(&document).expect("a persisted key");
    assert_eq!(key.expose_secret(), " sk-untouched ");
}

#[test]
fn master_key_from_config_returns_none_without_general_settings() {
    let document = GeneratedConfigDocument {
        model_list: Vec::new(),
        general_settings: None,
    };
    assert!(master_key_from_config(&document).is_none());
}

#[test]
fn master_key_from_config_returns_none_without_a_master_key() {
    let document = GeneratedConfigDocument {
        model_list: Vec::new(),
        general_settings: Some(GeneralSettings { master_key: None }),
    };
    assert!(master_key_from_config(&document).is_none());
}

#[rstest]
#[case::empty("")]
#[case::blank("   ")]
#[case::newlines("\n\t ")]
fn master_key_from_config_returns_none_for_a_blank_key(#[case] raw: &str) {
    let document = GeneratedConfigDocument {
        model_list: Vec::new(),
        general_settings: Some(GeneralSettings {
            master_key: Some(secret(raw)),
        }),
    };
    assert!(master_key_from_config(&document).is_none());
}

#[test]
fn master_key_from_config_reads_a_parsed_generated_document() {
    let yaml = "model_list: []\ngeneral_settings:\n  master_key: sk-persisted\n";
    let document: GeneratedConfigDocument =
        serde_saphyr::from_str(yaml).expect("a generated document");
    let key = master_key_from_config(&document).expect("a persisted key");
    assert_eq!(key.expose_secret(), "sk-persisted");
}

#[test]
fn generated_document_serializes_model_list_before_general_settings() {
    let document = GeneratedConfigDocument {
        model_list: build_generated_model_list(&config()),
        general_settings: Some(GeneralSettings {
            master_key: Some(secret("sk-persisted")),
        }),
    };
    let rendered = serde_saphyr::to_string(&document).expect("the generated document");
    let model_list = rendered.find("model_list:").expect("the model list");
    let general_settings = rendered
        .find("general_settings:")
        .expect("the general settings");
    assert!(model_list < general_settings);
    let router = rendered
        .find("complexity_router_config:")
        .expect("the router config");
    let tiers = rendered.find("tiers:").expect("the tiers");
    let default_model = rendered.find("default_model:").expect("the default model");
    assert!(router < tiers);
    assert!(tiers < default_model);
}

#[test]
fn generated_document_round_trips_through_yaml() {
    let document = GeneratedConfigDocument {
        model_list: build_generated_model_list(&config()),
        general_settings: Some(GeneralSettings {
            master_key: Some(secret("sk-persisted")),
        }),
    };
    let rendered = serde_saphyr::to_string(&document).expect("the generated document");
    let parsed: GeneratedConfigDocument = serde_saphyr::from_str(&rendered).expect("a round trip");
    assert_eq!(
        deployment_names(&parsed.model_list),
        deployment_names(&document.model_list)
    );
    let proxy = parsed
        .model_list
        .iter()
        .find_map(|deployment| match &deployment.litellm_params {
            DeploymentParams::Proxy(params) => Some(params),
            DeploymentParams::AutoRouter(_params) => None,
        })
        .expect("a proxy deployment");
    assert_eq!(proxy.api_key.expose_secret(), "sk-test");
    let key = master_key_from_config(&parsed).expect("the round-tripped key");
    assert_eq!(key.expose_secret(), "sk-persisted");
}

#[test]
fn generated_document_omits_general_settings_without_a_master_key() {
    let document = GeneratedConfigDocument {
        model_list: build_generated_model_list(&config()),
        general_settings: None,
    };
    let rendered = serde_saphyr::to_string(&document).expect("the generated document");
    assert!(!rendered.contains("general_settings"));
}
