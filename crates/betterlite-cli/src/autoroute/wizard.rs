pub mod error;
mod keywords;
mod persisted;
pub mod prompt;

mod tests;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use betterlite_logging::private_json::files::PrivateFiles;
use secrecy::SecretString;

use self::error::{ModelDiscoveryError, WizardError};
use self::keywords::prompt_keyword_tier_rules;
use self::persisted::{persisted_master_key, write_document};
use crate::autoroute::config::classifier::{
    ClassifierChoice, DEFAULT_CLASSIFIER_TIMEOUT_MS, LlmClassifierConfig,
};
use crate::autoroute::config::document::{
    GeneralSettings, GeneratedConfigDocument, build_generated_model_list,
};
use crate::autoroute::config::model::{
    DiscoveredModel, RawModelListing, chat_models, embedding_models, parse_discovered_models,
};
use crate::autoroute::config::semantic::{
    DEFAULT_MATCH_THRESHOLD, SemanticMatching, SemanticMatchingChoice,
};
use crate::autoroute::config::settings::{AutorouteConfig, validate_config};
use crate::autoroute::config::tier::Tier;
use crate::autoroute::paths;

const ASSIGN_TIERS_MESSAGE: &str =
    "Assign model(s) to each complexity tier (from what your key can access):";
const SELECT_AT_LEAST_ONE: &str = "Select at least one model.";
const LLM_CLASSIFIER_QUESTION: &str =
    "\nUse an LLM classifier instead of the free heuristic scorer?";
const LLM_CLASSIFIER_LABEL: &str = "LLM classifier";
const SEMANTIC_QUESTION: &str = "\nEnable semantic keyword matching?";
const SEMANTIC_LABEL: &str = "semantic embeddings";
const ADAPTIVE_QUESTION: &str = "\nEnable adaptive (bandit) selection on top of tiering?";

pub trait ModelDiscoveryClient {
    fn list_models(&self) -> Result<Vec<RawModelListing>, ModelDiscoveryError>;
}

pub trait Prompter {
    fn announce(&mut self, message: &str) -> Result<(), WizardError>;
    fn select_one(&mut self, message: &str, choices: &[String]) -> Result<String, WizardError>;
    fn select_many(
        &mut self,
        message: &str,
        choices: &[String],
    ) -> Result<Vec<String>, WizardError>;
    fn confirm(&mut self, message: &str, default: bool) -> Result<bool, WizardError>;
    fn prompt_keywords(&mut self, message: &str, default: &str) -> Result<String, WizardError>;
}

pub struct ConfigureRequest<'request> {
    pub home: &'request Path,
    pub base_url: String,
    pub api_key: SecretString,
    pub is_interactive: bool,
    pub client: &'request dyn ModelDiscoveryClient,
    pub prompter: &'request mut dyn Prompter,
    pub files: &'request PrivateFiles,
}

pub fn run_configure_wizard(request: ConfigureRequest<'_>) -> Result<PathBuf, WizardError> {
    let ConfigureRequest {
        home,
        base_url,
        api_key,
        is_interactive,
        client,
        prompter,
        files,
    } = request;
    let discovered = parse_discovered_models(&client.list_models()?);
    let chat_pool = chat_models(&discovered);
    let embedding_pool = embedding_models(&discovered);
    if chat_pool.is_empty() {
        return Err(WizardError::NoChatModels);
    }
    if !is_interactive {
        return Err(WizardError::NotInteractive);
    }
    let config = collect_config(prompter, base_url, api_key, &chat_pool, &embedding_pool)?;
    validate_config(&config, &discovered)?;
    let general_settings = persisted_master_key(home).map(|master_key| GeneralSettings {
        master_key: Some(master_key),
    });
    let document = GeneratedConfigDocument {
        model_list: build_generated_model_list(&config),
        general_settings,
    };
    write_document(files, home, &document)?;
    announce_written(prompter, home, &config.tiers)?;
    Ok(paths::config_path(home))
}

fn collect_config(
    prompter: &mut dyn Prompter,
    base_url: String,
    api_key: SecretString,
    chat_pool: &[DiscoveredModel],
    embedding_pool: &[DiscoveredModel],
) -> Result<AutorouteConfig, WizardError> {
    prompter.announce(ASSIGN_TIERS_MESSAGE)?;
    let tiers = prompt_tiers(prompter, chat_pool)?;
    let default_model = tiers
        .get(&Tier::Medium)
        .and_then(|models| models.first())
        .cloned()
        .ok_or(WizardError::NoDefaultModel)?;
    let classifier = prompt_classifier(prompter, chat_pool)?;
    let semantic_matching = prompt_semantic_matching(prompter, embedding_pool)?;
    let adaptive = prompter.confirm(ADAPTIVE_QUESTION, false)?;
    Ok(AutorouteConfig {
        base_url,
        api_key,
        tiers,
        default_model,
        classifier,
        semantic_matching,
        adaptive,
    })
}

fn prompt_tiers(
    prompter: &mut dyn Prompter,
    chat_pool: &[DiscoveredModel],
) -> Result<BTreeMap<Tier, Vec<String>>, WizardError> {
    let names: Vec<String> = chat_pool.iter().map(|model| model.name.clone()).collect();
    let mut tiers = BTreeMap::new();
    for tier in Tier::ALL {
        let message = format!("{tier}: type to filter, tab to toggle, enter to confirm");
        let mut selected = prompter.select_many(&message, &names)?;
        while selected.is_empty() {
            prompter.announce(SELECT_AT_LEAST_ONE)?;
            selected = prompter.select_many(&message, &names)?;
        }
        tiers.insert(tier, selected);
    }
    Ok(tiers)
}

fn prompt_model(
    prompter: &mut dyn Prompter,
    label: &str,
    pool: &[DiscoveredModel],
) -> Result<String, WizardError> {
    let message = format!("{label}: type to filter, enter to confirm");
    let names: Vec<String> = pool.iter().map(|model| model.name.clone()).collect();
    prompter.select_one(&message, &names)
}

fn prompt_classifier(
    prompter: &mut dyn Prompter,
    chat_pool: &[DiscoveredModel],
) -> Result<ClassifierChoice, WizardError> {
    if prompter.confirm(LLM_CLASSIFIER_QUESTION, false)? {
        let model = prompt_model(prompter, LLM_CLASSIFIER_LABEL, chat_pool)?;
        return Ok(ClassifierChoice::Llm(LlmClassifierConfig {
            model,
            timeout_ms: DEFAULT_CLASSIFIER_TIMEOUT_MS,
        }));
    }
    Ok(ClassifierChoice::Heuristic)
}

fn prompt_semantic_matching(
    prompter: &mut dyn Prompter,
    embedding_pool: &[DiscoveredModel],
) -> Result<SemanticMatchingChoice, WizardError> {
    if embedding_pool.is_empty() {
        return Ok(SemanticMatchingChoice::Disabled);
    }
    if !prompter.confirm(SEMANTIC_QUESTION, false)? {
        return Ok(SemanticMatchingChoice::Disabled);
    }
    let embedding_model = prompt_model(prompter, SEMANTIC_LABEL, embedding_pool)?;
    let keyword_tier_rules = prompt_keyword_tier_rules(prompter)?;
    Ok(SemanticMatchingChoice::Enabled(SemanticMatching {
        embedding_model,
        match_threshold: DEFAULT_MATCH_THRESHOLD,
        keyword_tier_rules,
    }))
}

fn announce_written(
    prompter: &mut dyn Prompter,
    home: &Path,
    tiers: &BTreeMap<Tier, Vec<String>>,
) -> Result<(), WizardError> {
    let path = paths::config_path(home);
    prompter.announce(&format!("\nWrote {}", path.display()))?;
    for (tier, models) in tiers {
        prompter.announce(&format!("  {tier}: {}", models.join(", ")))?;
    }
    Ok(())
}
