use std::collections::{BTreeMap, BTreeSet};

use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};

use super::AUTOROUTER_MODEL_NAME;
use super::classifier::{ClassifierChoice, ClassifierType, LlmClassifierConfig};
use super::semantic::{KeywordTierRule, SemanticMatchingChoice};
use super::settings::AutorouteConfig;
use super::tier::Tier;

const AUTO_ROUTER_MODEL: &str = "auto_router/complexity_router";
const PROXY_MODEL_PREFIX: &str = "litellm_proxy/";

// the wire form of the key exists only for the config file write; this struct derives no Debug
#[derive(Deserialize, Serialize)]
pub struct ProxyParams {
    pub model: String,
    pub api_base: String,
    pub api_key: String,
}

#[derive(Deserialize, Serialize)]
pub struct AutoRouterParams {
    pub model: String,
    pub complexity_router_config: ComplexityRouterConfig,
}

#[derive(Deserialize, Serialize)]
#[serde(untagged)]
pub enum DeploymentParams {
    Proxy(ProxyParams),
    AutoRouter(AutoRouterParams),
}

#[derive(Deserialize, Serialize)]
pub struct ModelDeployment {
    pub model_name: String,
    pub litellm_params: DeploymentParams,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ComplexityRouterConfig {
    #[serde(default)]
    pub tiers: BTreeMap<Tier, Vec<String>>,
    #[serde(default)]
    pub default_model: String,
    #[serde(default)]
    pub return_raw_model_name: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classifier_type: Option<ClassifierType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classifier_llm_config: Option<LlmClassifierConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_keyword_matching: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub match_threshold: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyword_tier_rules: Option<Vec<KeywordTierRule>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adaptive: Option<bool>,
}

#[derive(Deserialize, Serialize)]
pub struct GeneralSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub master_key: Option<String>,
}

#[derive(Deserialize, Serialize)]
pub struct GeneratedConfigDocument {
    #[serde(default)]
    pub model_list: Vec<ModelDeployment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub general_settings: Option<GeneralSettings>,
}

#[must_use]
pub fn build_generated_model_list(config: &AutorouteConfig) -> Vec<ModelDeployment> {
    let mut referenced: BTreeSet<&str> = BTreeSet::new();
    for models in config.tiers.values() {
        for model in models {
            referenced.insert(model.as_str());
        }
    }
    referenced.insert(config.default_model.as_str());
    if let ClassifierChoice::Llm(classifier) = &config.classifier {
        referenced.insert(classifier.model.as_str());
    }
    if let SemanticMatchingChoice::Enabled(semantic) = &config.semantic_matching {
        referenced.insert(semantic.embedding_model.as_str());
    }

    let mut deployments: Vec<ModelDeployment> = referenced
        .into_iter()
        .map(|name| ModelDeployment {
            model_name: name.to_owned(),
            litellm_params: DeploymentParams::Proxy(ProxyParams {
                model: format!("{PROXY_MODEL_PREFIX}{name}"),
                api_base: config.base_url.clone(),
                api_key: config.api_key.expose_secret().to_owned(),
            }),
        })
        .collect();
    deployments.push(ModelDeployment {
        model_name: AUTOROUTER_MODEL_NAME.to_owned(),
        litellm_params: DeploymentParams::AutoRouter(AutoRouterParams {
            model: AUTO_ROUTER_MODEL.to_owned(),
            complexity_router_config: complexity_router_config(config),
        }),
    });
    deployments
}

fn complexity_router_config(config: &AutorouteConfig) -> ComplexityRouterConfig {
    let mut router = ComplexityRouterConfig {
        tiers: config.tiers.clone(),
        default_model: config.default_model.clone(),
        return_raw_model_name: true,
        classifier_type: None,
        classifier_llm_config: None,
        semantic_keyword_matching: None,
        embedding_model: None,
        match_threshold: None,
        keyword_tier_rules: None,
        adaptive: config.adaptive.then_some(true),
    };
    if let ClassifierChoice::Llm(classifier) = &config.classifier {
        router.classifier_type = Some(ClassifierType::Llm);
        router.classifier_llm_config = Some(classifier.clone());
    }
    if let SemanticMatchingChoice::Enabled(semantic) = &config.semantic_matching {
        router.semantic_keyword_matching = Some(true);
        router.embedding_model = Some(semantic.embedding_model.clone());
        router.match_threshold = Some(semantic.match_threshold);
        router.keyword_tier_rules = Some(semantic.keyword_tier_rules.clone());
    }
    router
}

#[must_use]
pub fn master_key_from_config(document: &GeneratedConfigDocument) -> Option<SecretString> {
    let master_key = document.general_settings.as_ref()?.master_key.as_ref()?;
    // LiteLLM reads the key through master_key.strip(), so blank means absent while a real key stays verbatim
    if master_key.trim().is_empty() {
        return None;
    }
    Some(SecretString::from(master_key.clone()))
}
