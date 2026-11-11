#![cfg(test)]

use std::collections::VecDeque;
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use betterlite_logging::private_json::files::{FileAccess, PrivateFiles};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tempfile::TempDir;

use super::error::WizardError;
use super::keywords::parse_keywords;
use super::{ConfigureRequest, run_configure_wizard};
use crate::autoroute::config::AUTOROUTER_MODEL_NAME;
use crate::autoroute::config::document::{
    ComplexityRouterConfig, DeploymentParams, GeneratedConfigDocument,
};
use crate::autoroute::config::error::ConfigError;
use crate::autoroute::config::model::RawModelListing;
use crate::autoroute::config::semantic::KeywordTierRule;
use crate::autoroute::config::tier::Tier;
use crate::autoroute::paths;

mod support;

use support::{FailingDirectory, FakeClient, FakePrompter};

fn listing(name: &str, mode: &str) -> RawModelListing {
    RawModelListing {
        id: name.to_owned(),
        mode: mode.to_owned(),
    }
}

fn chat(name: &str) -> RawModelListing {
    listing(name, "chat")
}

fn embedding(name: &str) -> RawModelListing {
    listing(name, "embedding")
}

fn selections(entries: &[&[&str]]) -> VecDeque<Vec<String>> {
    entries
        .iter()
        .map(|models| models.iter().map(|name| (*name).to_owned()).collect())
        .collect()
}

fn texts(entries: &[&str]) -> VecDeque<String> {
    entries.iter().map(|text| (*text).to_owned()).collect()
}

fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

fn target_path(home: &Path) -> PathBuf {
    paths::config_path(home)
}

fn read_document(home: &Path) -> GeneratedConfigDocument {
    let text = fs::read_to_string(target_path(home)).expect("the written config");
    serde_saphyr::from_str(&text).expect("the generated document")
}

fn complexity_router(document: &GeneratedConfigDocument) -> ComplexityRouterConfig {
    let deployment = document
        .model_list
        .iter()
        .find(|deployment| deployment.model_name == AUTOROUTER_MODEL_NAME)
        .expect("the autorouter deployment");
    let DeploymentParams::AutoRouter(params) = &deployment.litellm_params else {
        panic!("the autorouter deployment carries the auto router params");
    };
    params.complexity_router_config.clone()
}

fn deployment_names(document: &GeneratedConfigDocument) -> Vec<&str> {
    document
        .model_list
        .iter()
        .map(|deployment| deployment.model_name.as_str())
        .collect()
}

fn mode(path: &Path) -> u32 {
    fs::metadata(path)
        .expect("the path metadata")
        .permissions()
        .mode()
        & 0o777
}

fn run(
    home: &Path,
    client: &FakeClient,
    prompter: &mut FakePrompter,
    files: &PrivateFiles,
    is_interactive: bool,
) -> Result<PathBuf, WizardError> {
    run_configure_wizard(ConfigureRequest {
        home,
        base_url: "https://proxy.example".to_owned(),
        api_key: secret("sk-test"),
        is_interactive,
        client,
        prompter,
        files,
    })
}

fn rule_list(rules: &[KeywordTierRule]) -> Vec<(Tier, Vec<&str>)> {
    rules
        .iter()
        .map(|rule| {
            (
                rule.tier,
                rule.keywords.iter().map(String::as_str).collect(),
            )
        })
        .collect()
}

fn assert_tier_prompts_in_order(prompter: &FakePrompter) {
    let messages = &prompter.many_messages;
    assert_eq!(messages.len(), 4);
    assert_eq!(prompter.many_choices.len(), 4);
    for (index, tier) in Tier::ALL.iter().enumerate() {
        assert_eq!(
            messages[index],
            format!("{tier}: type to filter, tab to toggle, enter to confirm")
        );
        assert_eq!(prompter.many_choices[index], vec!["alpha", "beta"]);
    }
}

fn assert_written_announcements(prompter: &FakePrompter) {
    let announcements = &prompter.announcements;
    assert!(announcements.iter().any(|message| message
        == "Assign model(s) to each complexity tier (from what your key can access):"));
    assert!(
        announcements
            .iter()
            .any(|message| message.ends_with("  SIMPLE: beta"))
    );
    assert!(
        announcements
            .iter()
            .any(|message| message.ends_with("  MEDIUM: alpha, beta"))
    );
    assert!(
        announcements
            .iter()
            .any(|message| message.contains("Wrote"))
    );
}

#[test]
fn run_configure_wizard_discovers_models_and_builds_tiers_in_tier_order() {
    let home = TempDir::new().expect("a home");
    let client = FakeClient::new(vec![
        chat("alpha"),
        embedding("embed"),
        chat("beta"),
        listing("gamma", "responses"),
    ]);
    let mut prompter = FakePrompter::new();
    prompter.selections = selections(&[&["beta"], &["alpha", "beta"], &["beta"], &["alpha"]]);
    prompter.confirm_answers = VecDeque::from([false, false, false]);
    let files = PrivateFiles::default();

    let path = run(home.path(), &client, &mut prompter, &files, true).expect("a written config");

    assert_eq!(path, target_path(home.path()));
    assert_eq!(client.calls(), 1);
    assert_tier_prompts_in_order(&prompter);
    assert_written_announcements(&prompter);
    let document = read_document(home.path());
    assert_eq!(
        deployment_names(&document),
        vec!["alpha", "beta", AUTOROUTER_MODEL_NAME]
    );
    let router = complexity_router(&document);
    assert_eq!(router.default_model, "alpha");
    assert!(router.return_raw_model_name);
    assert!(!router.adaptive);
    assert_eq!(
        router.tiers.get(&Tier::Simple),
        Some(&vec!["beta".to_owned()])
    );
    assert_eq!(
        router.tiers.get(&Tier::Medium),
        Some(&vec!["alpha".to_owned(), "beta".to_owned()])
    );
    assert!(document.general_settings.is_none());
}

fn assert_classifier_and_semantic_prompts(prompter: &FakePrompter) {
    assert_eq!(prompter.one_messages.len(), 2);
    assert_eq!(
        prompter.one_messages[0],
        "LLM classifier: type to filter, enter to confirm"
    );
    assert_eq!(prompter.one_choices[0], vec!["alpha", "beta"]);
    assert_eq!(
        prompter.one_messages[1],
        "semantic embeddings: type to filter, enter to confirm"
    );
    assert_eq!(prompter.one_choices[1], vec!["embed-a", "embed-b"]);
    let keywords: Vec<(&str, &str)> = prompter
        .keyword_calls
        .iter()
        .map(|(message, default)| (message.as_str(), default.as_str()))
        .collect();
    assert_eq!(
        keywords,
        vec![
            ("  SIMPLE keywords", "hi, hello, thanks"),
            ("  MEDIUM keywords", "explain, how does"),
            ("  COMPLEX keywords", "refactor, implement, debug"),
            ("  REASONING keywords", "step by step, think through, prove"),
        ]
    );
    let confirms = &prompter.confirm_calls;
    assert_eq!(confirms.len(), 3);
    assert_eq!(
        confirms[0].0,
        "\nUse an LLM classifier instead of the free heuristic scorer?"
    );
    assert!(!confirms[0].1);
    assert_eq!(confirms[1].0, "\nEnable semantic keyword matching?");
    assert!(!confirms[1].1);
    assert_eq!(
        confirms[2].0,
        "\nEnable adaptive (bandit) selection on top of tiering?"
    );
    assert!(!confirms[2].1);
}

fn assert_classifier_and_semantic_settings(router: &ComplexityRouterConfig) {
    assert_eq!(
        router
            .classifier_llm_config
            .as_ref()
            .map(|config| config.model.as_str()),
        Some("beta")
    );
    assert_eq!(
        router
            .classifier_llm_config
            .as_ref()
            .map(|config| config.timeout_ms),
        Some(3000)
    );
    assert!(router.semantic.semantic_keyword_matching);
    assert_eq!(router.semantic.embedding_model.as_deref(), Some("embed-b"));
    assert_eq!(router.semantic.match_threshold, Some(0.5_f64));
    assert_eq!(
        rule_list(
            router
                .semantic
                .keyword_tier_rules
                .as_deref()
                .expect("keyword rules")
        ),
        vec![
            (Tier::Simple, vec!["hi", "hello", "thanks"]),
            (Tier::Medium, vec!["custom", "explain"]),
            (Tier::Complex, vec!["refactor"]),
            (Tier::Reasoning, vec!["prove", "step by step"]),
        ]
    );
    assert!(router.adaptive);
}

#[test]
fn run_configure_wizard_prompts_llm_classifier_semantic_keywords_and_adaptive() {
    let home = TempDir::new().expect("a home");
    let client = FakeClient::new(vec![
        chat("alpha"),
        embedding("embed-a"),
        chat("beta"),
        embedding("embed-b"),
    ]);
    let mut prompter = FakePrompter::new();
    prompter.selections = selections(&[&["alpha"], &["beta"], &["alpha"], &["beta"]]);
    prompter.confirm_answers = VecDeque::from([true, true, true]);
    prompter.singles = texts(&["beta", "embed-b"]);
    prompter.keyword_answers =
        texts(&["", " custom , ,explain ", "refactor", "prove,step by step"]);
    let files = PrivateFiles::default();

    run(home.path(), &client, &mut prompter, &files, true).expect("a written config");

    assert_classifier_and_semantic_prompts(&prompter);
    let document = read_document(home.path());
    assert_eq!(
        deployment_names(&document),
        vec!["alpha", "beta", "embed-b", AUTOROUTER_MODEL_NAME]
    );
    assert_classifier_and_semantic_settings(&complexity_router(&document));
}

#[test]
fn run_configure_wizard_skips_semantic_prompts_without_embedding_models() {
    let home = TempDir::new().expect("a home");
    let client = FakeClient::new(vec![chat("alpha"), chat("beta")]);
    let mut prompter = FakePrompter::new();
    prompter.selections = selections(&[&["alpha"], &["alpha"], &["alpha"], &["alpha"]]);
    prompter.confirm_answers = VecDeque::from([false, false]);
    let files = PrivateFiles::default();

    run(home.path(), &client, &mut prompter, &files, true).expect("a written config");

    let confirms = &prompter.confirm_calls;
    assert_eq!(confirms.len(), 2);
    assert_eq!(
        confirms[0].0,
        "\nUse an LLM classifier instead of the free heuristic scorer?"
    );
    assert_eq!(
        confirms[1].0,
        "\nEnable adaptive (bandit) selection on top of tiering?"
    );
    let router = complexity_router(&read_document(home.path()));
    assert!(!router.semantic.semantic_keyword_matching);
    assert_eq!(router.classifier_type, None);
}

#[rstest]
#[case::two("alpha,beta", vec!["alpha", "beta"])]
#[case::spaced(" alpha , beta ", vec!["alpha", "beta"])]
#[case::empty_parts("alpha,,beta,", vec!["alpha", "beta"])]
#[case::blank("", Vec::new())]
#[case::separators(" , , ", Vec::new())]
#[case::phrase("step by step", vec!["step by step"])]
fn parse_keywords_splits_on_commas_trims_parts_and_drops_empty_ones(
    #[case] raw: &str,
    #[case] expected: Vec<&str>,
) {
    assert_eq!(parse_keywords(raw), expected);
}

#[test]
fn run_configure_wizard_reprompts_each_tier_until_a_model_is_selected() {
    let home = TempDir::new().expect("a home");
    let client = FakeClient::new(vec![chat("alpha")]);
    let mut prompter = FakePrompter::new();
    prompter.selections = selections(&[&[], &["alpha"], &["alpha"], &["alpha"], &["alpha"]]);
    prompter.confirm_answers = VecDeque::from([false, false]);
    let files = PrivateFiles::default();

    run(home.path(), &client, &mut prompter, &files, true).expect("a written config");

    let messages = &prompter.many_messages;
    assert_eq!(messages.len(), 5);
    assert_eq!(
        messages[0],
        "SIMPLE: type to filter, tab to toggle, enter to confirm"
    );
    assert_eq!(messages[1], messages[0]);
    assert_eq!(
        prompter
            .announcements
            .iter()
            .filter(|message| **message == "Select at least one model.")
            .count(),
        1
    );
    let document = read_document(home.path());
    let router = complexity_router(&document);
    assert_eq!(
        router.tiers.get(&Tier::Simple),
        Some(&vec!["alpha".to_owned()])
    );
}

#[test]
fn run_configure_wizard_reports_no_chat_models_before_the_interactive_check() {
    let home = TempDir::new().expect("a home");
    let client = FakeClient::new(vec![embedding("embed")]);
    let mut prompter = FakePrompter::new();
    let files = PrivateFiles::default();

    let error =
        run(home.path(), &client, &mut prompter, &files, false).expect_err("no chat models");

    assert!(matches!(error, WizardError::NoChatModels));
    assert_eq!(client.calls(), 1);
    assert!(!prompter.was_prompted());
    assert!(!target_path(home.path()).exists());
}

#[test]
fn run_configure_wizard_refuses_a_non_interactive_run_before_prompting() {
    let home = TempDir::new().expect("a home");
    let client = FakeClient::new(vec![chat("alpha")]);
    let mut prompter = FakePrompter::new();
    let files = PrivateFiles::default();

    let error =
        run(home.path(), &client, &mut prompter, &files, false).expect_err("not a terminal");

    assert!(matches!(error, WizardError::NotInteractive));
    assert!(!prompter.was_prompted());
    assert!(!target_path(home.path()).exists());
}

#[test]
fn run_configure_wizard_writes_the_config_with_owner_only_modes() {
    let home = TempDir::new().expect("a home");
    let client = FakeClient::new(vec![chat("alpha")]);
    let mut prompter = FakePrompter::new();
    prompter.selections = selections(&[&["alpha"], &["alpha"], &["alpha"], &["alpha"]]);
    prompter.confirm_answers = VecDeque::from([false, false]);
    let files = PrivateFiles::default();

    let path = run(home.path(), &client, &mut prompter, &files, true).expect("a written config");

    assert_eq!(mode(&paths::autoroute_dir(home.path())), 0o700);
    assert_eq!(mode(&path), 0o600);
    assert!(read_document(home.path()).general_settings.is_none());
}

#[test]
fn run_configure_wizard_writes_a_persisted_master_key_into_general_settings() {
    let home = TempDir::new().expect("a home");
    fs::create_dir_all(paths::autoroute_dir(home.path())).expect("the autoroute directory");
    fs::write(
        target_path(home.path()),
        "model_list: []\ngeneral_settings:\n  master_key: sk-old\n",
    )
    .expect("a persisted config");
    let client = FakeClient::new(vec![chat("alpha")]);
    let mut prompter = FakePrompter::new();
    prompter.selections = selections(&[&["alpha"], &["alpha"], &["alpha"], &["alpha"]]);
    prompter.confirm_answers = VecDeque::from([false, false]);
    let files = PrivateFiles::default();

    run(home.path(), &client, &mut prompter, &files, true).expect("a rewritten config");

    let document = read_document(home.path());
    let settings = document
        .general_settings
        .as_ref()
        .expect("the general settings");
    let key = settings.master_key.as_ref().expect("the persisted key");
    assert_eq!(key.expose_secret(), "sk-old");
    assert_eq!(
        deployment_names(&document),
        vec!["alpha", AUTOROUTER_MODEL_NAME]
    );
}

#[test]
fn run_configure_wizard_writes_no_general_settings_without_a_persisted_key() {
    let home = TempDir::new().expect("a home");
    let client = FakeClient::new(vec![chat("alpha")]);
    let mut prompter = FakePrompter::new();
    prompter.selections = selections(&[&["alpha"], &["alpha"], &["alpha"], &["alpha"]]);
    prompter.confirm_answers = VecDeque::from([false, false]);
    let files = PrivateFiles::default();

    run(home.path(), &client, &mut prompter, &files, true).expect("a written config");

    let text = fs::read_to_string(target_path(home.path())).expect("the written config");
    assert!(!text.contains("general_settings"));
    assert!(!text.contains("master_key"));
}

#[test]
fn run_configure_wizard_ignores_a_corrupt_persisted_config() {
    let home = TempDir::new().expect("a home");
    fs::create_dir_all(paths::autoroute_dir(home.path())).expect("the autoroute directory");
    fs::write(target_path(home.path()), "model_list: [").expect("a corrupt config");
    let client = FakeClient::new(vec![chat("alpha")]);
    let mut prompter = FakePrompter::new();
    prompter.selections = selections(&[&["alpha"], &["alpha"], &["alpha"], &["alpha"]]);
    prompter.confirm_answers = VecDeque::from([false, false]);
    let files = PrivateFiles::default();

    run(home.path(), &client, &mut prompter, &files, true).expect("a rewritten config");

    assert!(read_document(home.path()).general_settings.is_none());
}

#[test]
fn run_configure_wizard_does_not_write_when_the_directory_cannot_be_prepared() {
    let home = TempDir::new().expect("a home");
    let client = FakeClient::new(vec![chat("alpha")]);
    let mut prompter = FakePrompter::new();
    prompter.selections = selections(&[&["alpha"], &["alpha"], &["alpha"], &["alpha"]]);
    prompter.confirm_answers = VecDeque::from([false, false]);
    let access: Arc<dyn FileAccess> = Arc::new(FailingDirectory::new());
    let files = PrivateFiles::from(access);

    let error =
        run(home.path(), &client, &mut prompter, &files, true).expect_err("a failing directory");

    assert!(matches!(error, WizardError::Directory(_)));
    assert!(!target_path(home.path()).exists());
}

#[test]
fn run_configure_wizard_does_not_write_an_invalid_config() {
    let home = TempDir::new().expect("a home");
    let client = FakeClient::new(vec![chat("alpha")]);
    let mut prompter = FakePrompter::new();
    prompter.selections = selections(&[&["ghost"], &["ghost"], &["ghost"], &["ghost"]]);
    prompter.confirm_answers = VecDeque::from([false, false]);
    let files = PrivateFiles::default();

    let error =
        run(home.path(), &client, &mut prompter, &files, true).expect_err("an invalid config");

    assert!(matches!(
        error,
        WizardError::Config(ConfigError::TierModelNotDiscovered { .. })
    ));
    assert!(!target_path(home.path()).exists());
}

#[test]
fn run_configure_wizard_returns_the_discovery_error_without_prompting() {
    let home = TempDir::new().expect("a home");
    let client = FakeClient::failing();
    let mut prompter = FakePrompter::new();
    let files = PrivateFiles::default();

    let error =
        run(home.path(), &client, &mut prompter, &files, true).expect_err("a failed listing");

    assert!(matches!(error, WizardError::Discovery(_)));
    assert!(!prompter.was_prompted());
}

#[test]
fn run_configure_wizard_returns_a_prompt_error() {
    let home = TempDir::new().expect("a home");
    let client = FakeClient::new(vec![chat("alpha")]);
    let mut prompter = FakePrompter::new();
    prompter.fail = true;
    let files = PrivateFiles::default();

    let error =
        run(home.path(), &client, &mut prompter, &files, true).expect_err("a failed prompt");

    assert!(matches!(error, WizardError::Prompt(_)));
    assert!(!target_path(home.path()).exists());
}

#[test]
fn run_configure_wizard_keeps_every_tier_in_a_single_deployment() {
    let home = TempDir::new().expect("a home");
    let client = FakeClient::new(vec![chat("alpha"), chat("beta")]);
    let mut prompter = FakePrompter::new();
    prompter.selections = selections(&[&["alpha"], &["alpha"], &["alpha"], &["alpha"]]);
    prompter.confirm_answers = VecDeque::from([false, false]);
    let files = PrivateFiles::default();

    run(home.path(), &client, &mut prompter, &files, true).expect("a written config");

    let document = read_document(home.path());
    assert_eq!(
        deployment_names(&document),
        vec!["alpha", AUTOROUTER_MODEL_NAME]
    );
    let router = complexity_router(&document);
    assert_eq!(router.tiers.len(), 4);
    assert_eq!(
        router.tiers.get(&Tier::Simple),
        Some(&vec!["alpha".to_owned()])
    );
    assert_eq!(
        router.tiers.get(&Tier::Medium),
        Some(&vec!["alpha".to_owned()])
    );
    assert_eq!(
        router.tiers.get(&Tier::Complex),
        Some(&vec!["alpha".to_owned()])
    );
    assert_eq!(
        router.tiers.get(&Tier::Reasoning),
        Some(&vec!["alpha".to_owned()])
    );
}
