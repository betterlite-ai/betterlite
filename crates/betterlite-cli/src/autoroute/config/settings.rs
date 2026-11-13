use std::collections::{BTreeMap, BTreeSet};

use secrecy::SecretString;

use super::classifier::ClassifierChoice;
use super::error::ConfigError;
use super::model::{DiscoveredModel, chat_models, embedding_models};
use super::semantic::SemanticMatchingChoice;
use super::tier::Tier;

#[derive(Clone, Debug)]
pub struct AutorouteConfig {
    pub base_url: String,
    pub api_key: SecretString,
    pub tiers: BTreeMap<Tier, Vec<String>>,
    pub default_model: String,
    pub classifier: ClassifierChoice,
    pub semantic_matching: SemanticMatchingChoice,
    pub adaptive: bool,
}

pub fn validate_config(
    config: &AutorouteConfig,
    discovered: &[DiscoveredModel],
) -> Result<(), ConfigError> {
    let chat_pool = chat_models(discovered);
    let chat_names: BTreeSet<&str> = chat_pool.iter().map(|model| model.name.as_str()).collect();
    let embedding_pool = embedding_models(discovered);
    let embedding_names: BTreeSet<&str> = embedding_pool
        .iter()
        .map(|model| model.name.as_str())
        .collect();

    for (tier, models) in &config.tiers {
        for model in models {
            if !chat_names.contains(model.as_str()) {
                return Err(ConfigError::TierModelNotDiscovered {
                    tier: *tier,
                    model: model.clone(),
                });
            }
        }
    }

    if !chat_names.contains(config.default_model.as_str()) {
        return Err(ConfigError::DefaultModelNotDiscovered {
            model: config.default_model.clone(),
        });
    }

    if let ClassifierChoice::Llm(classifier) = &config.classifier
        && !chat_names.contains(classifier.model.as_str())
    {
        return Err(ConfigError::ClassifierModelNotDiscovered {
            model: classifier.model.clone(),
        });
    }

    if let SemanticMatchingChoice::Enabled(semantic) = &config.semantic_matching
        && !embedding_names.contains(semantic.embedding_model.as_str())
    {
        return Err(ConfigError::EmbeddingModelNotDiscovered {
            model: semantic.embedding_model.clone(),
        });
    }

    Ok(())
}
