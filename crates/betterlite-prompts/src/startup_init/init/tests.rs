#![cfg(test)]

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::path::Path;
use std::sync::{Arc, Mutex};

use betterlite_config::loader::ConfigLoader;
use betterlite_models::decisions::Json;
use betterlite_models::prompt_contracts::PromptManagementClient;
use indexmap::IndexMap;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, Subscriber, set_default};
use tracing::{Event, Level};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::registry::mutation::prompt_registry_mutation::callbacks::PromptCallbackManager;
use crate::registry::mutation::prompt_registry_mutation::methods::PromptRegistryMutation as _;
use crate::registry::prompt_registry::InMemoryPromptRegistry;
use crate::startup_init::error::InitPromptsError;
use crate::startup_init::init::init_prompts;
use crate::tests::prompt_spec;

#[derive(Default)]
struct RecordingCallbacks {
    added: Vec<PromptManagementClient>,
}

impl PromptCallbackManager for RecordingCallbacks {
    fn add_litellm_callback(&mut self, callback: &PromptManagementClient) {
        self.added.push(callback.clone());
    }

    fn remove_callback_from_all_lists(&mut self, _callback: &PromptManagementClient) {}
}

#[derive(Clone, Default)]
struct CapturedEvents(Arc<Mutex<Vec<String>>>);

#[derive(Default)]
struct MessageFields(Vec<String>);

impl Visit for MessageFields {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0.push(format!("{value:?}"));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.0.push(value.to_owned());
        }
    }
}

impl<S: Subscriber> Layer<S> for CapturedEvents {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        if *event.metadata().level() != Level::DEBUG {
            return;
        }
        let mut fields = MessageFields::default();
        event.record(&mut fields);
        if let Ok(mut events) = self.0.lock() {
            events.extend(fields.0);
        }
    }
}

fn capture_debug_messages() -> (Arc<Mutex<Vec<String>>>, DefaultGuard) {
    let messages = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(CapturedEvents(Arc::clone(&messages)));
    (messages, set_default(subscriber))
}

fn prompt_list_logs(messages: &Arc<Mutex<Vec<String>>>) -> usize {
    let captured = messages.lock().expect("the capture is not poisoned");
    captured
        .iter()
        .filter(|message| message.contains("Prompt List:"))
        .count()
}

fn entry(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).expect("the test entry is a mapping")
}

fn entries(texts: &[&str]) -> Vec<IndexMap<String, Json>> {
    texts.iter().map(|text| entry(text)).collect()
}

#[test]
fn registers_every_entry_in_input_order() {
    let all = entries(&[
        r#"{"prompt_id":"first","litellm_params":{"prompt_integration":"dotprompt"}}"#,
        r#"{"prompt_id":"second","environment":"production","litellm_params":{"prompt_integration":"gitlab"}}"#,
    ]);
    let mut registry = InMemoryPromptRegistry::default();
    let mut callbacks = RecordingCallbacks::default();
    let registered =
        init_prompts(&mut registry, &all, None, &mut callbacks).expect("both prompts register");
    assert_eq!(
        registered
            .iter()
            .map(|spec| spec.prompt_id.as_str())
            .collect::<Vec<_>>(),
        ["first", "second"]
    );
    assert_eq!(
        registry
            .in_memory_prompts
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["first::development", "second::production"]
    );
    assert_eq!(callbacks.added.len(), 2);
}

#[test]
fn an_entry_without_prompt_info_registers_with_the_defaults_litellm_injects() {
    let all = entries(&[
        r#"{"prompt_id":"greeting","litellm_params":{"prompt_integration":"dotprompt"}}"#,
    ]);
    let mut registry = InMemoryPromptRegistry::default();
    let mut callbacks = RecordingCallbacks::default();
    let registered =
        init_prompts(&mut registry, &all, None, &mut callbacks).expect("the prompt registers");
    assert_eq!(registered.len(), 1);
    assert_eq!(registered[0].prompt_info.prompt_type, "config");
    assert_eq!(registered[0].prompt_info.environment, "development");
    assert_eq!(registered[0].environment, "development");
    assert!(
        registry
            .in_memory_prompts
            .contains_key("greeting::development")
    );
}

#[test]
fn prompt_info_without_a_prompt_type_gets_the_config_default() {
    let all = entries(&[
        r#"{"prompt_id":"greeting","prompt_info":{"environment":"staging"},"litellm_params":{"prompt_integration":"dotprompt"}}"#,
    ]);
    let mut registry = InMemoryPromptRegistry::default();
    let mut callbacks = RecordingCallbacks::default();
    let registered =
        init_prompts(&mut registry, &all, None, &mut callbacks).expect("the prompt registers");
    assert_eq!(registered.len(), 1);
    assert_eq!(registered[0].prompt_info.prompt_type, "config");
    assert_eq!(registered[0].prompt_info.environment, "staging");
}

#[test]
fn an_entry_with_typed_environment_references_registers() {
    let source = "prompts:\n  - prompt_id: greeting\n    version: os.environ/PROMPT_VERSION\n    litellm_params:\n      prompt_integration: dotprompt\n      ignore_prompt_manager_model: os.environ/IGNORE_MODEL\n      ignore_prompt_manager_optional_params: os.environ/IGNORE_PARAMS\n";
    let lookup = BTreeMap::from([
        ("PROMPT_VERSION".to_owned(), "2".to_owned()),
        ("IGNORE_MODEL".to_owned(), "true".to_owned()),
        ("IGNORE_PARAMS".to_owned(), "false".to_owned()),
    ]);
    let config = ConfigLoader { lookup }
        .load(source)
        .expect("the config loads");
    let mut registry = InMemoryPromptRegistry::default();
    let mut callbacks = RecordingCallbacks::default();
    let registered = init_prompts(
        &mut registry,
        &config.sections.prompts.prompts,
        None,
        &mut callbacks,
    )
    .expect("the prompt registers");
    assert_eq!(registered.len(), 1);
    assert_eq!(registered[0].version, Some(2));
    assert!(registered[0].litellm_params.ignore_model);
    assert!(!registered[0].litellm_params.ignore_optional_params);
    assert!(
        registry
            .in_memory_prompts
            .contains_key("greeting::development")
    );
}

#[test]
fn an_entry_with_unset_flag_references_registers_with_the_flags_off() {
    let source = "prompts:\n  - prompt_id: greeting\n    litellm_params:\n      prompt_integration: dotprompt\n      ignore_prompt_manager_model: os.environ/IGNORE_MODEL\n      ignore_prompt_manager_optional_params: os.environ/IGNORE_PARAMS\n";
    let lookup = BTreeMap::from([("IGNORE_PARAMS".to_owned(), "y".to_owned())]);
    let config = ConfigLoader { lookup }
        .load(source)
        .expect("the config loads");
    let mut registry = InMemoryPromptRegistry::default();
    let mut callbacks = RecordingCallbacks::default();
    let registered = init_prompts(
        &mut registry,
        &config.sections.prompts.prompts,
        None,
        &mut callbacks,
    )
    .expect("the prompt registers");
    assert_eq!(registered.len(), 1);
    assert!(!registered[0].litellm_params.ignore_model);
    assert!(registered[0].litellm_params.ignore_optional_params);
}

#[test]
fn a_prompt_already_registered_comes_back_as_the_stored_spec() {
    let mut stored = prompt_spec("greeting", Some(1), "development");
    stored.litellm_params.dotprompt_content = Some("Original".to_owned());
    let mut registry = InMemoryPromptRegistry::default();
    let mut callbacks = RecordingCallbacks::default();
    registry
        .initialize_prompt(&stored, None, &mut callbacks)
        .expect("the fixture prompt registers");
    callbacks.added.clear();
    let all = entries(&[
        r#"{"prompt_id":"greeting","version":2,"litellm_params":{"prompt_integration":"dotprompt","dotprompt_content":"Changed"}}"#,
    ]);
    let registered = init_prompts(&mut registry, &all, None, &mut callbacks)
        .expect("the stored prompt comes back");
    assert_eq!(registered.len(), 1);
    assert_eq!(registered[0].version, Some(1));
    assert_eq!(
        registered[0].litellm_params.dotprompt_content.as_deref(),
        Some("Original")
    );
    assert_eq!(registry.in_memory_prompts.len(), 1);
    assert!(callbacks.added.is_empty());
}

#[test]
fn a_bad_entry_stops_at_its_index_and_keeps_the_earlier_entries() {
    let all = entries(&[
        r#"{"prompt_id":"first","litellm_params":{"prompt_integration":"dotprompt"}}"#,
        r#"{"prompt_id":5,"litellm_params":{"prompt_integration":"dotprompt"}}"#,
    ]);
    let mut registry = InMemoryPromptRegistry::default();
    let mut callbacks = RecordingCallbacks::default();
    let error = init_prompts(&mut registry, &all, None, &mut callbacks)
        .expect_err("the second entry is not a prompt specification");
    assert!(matches!(
        error,
        InitPromptsError::InvalidPrompt { index: 1, .. }
    ));
    assert_eq!(registry.in_memory_prompts.len(), 1);
    assert!(
        registry
            .in_memory_prompts
            .contains_key("first::development")
    );
    assert_eq!(callbacks.added.len(), 1);
}

#[test]
fn an_initialize_error_stops_at_that_prompt_and_keeps_the_earlier_entries() {
    let all = entries(&[
        r#"{"prompt_id":"first","litellm_params":{"prompt_integration":"dotprompt"}}"#,
        r#"{"prompt_id":"second","litellm_params":{"prompt_integration":"langfuse"}}"#,
    ]);
    let mut registry = InMemoryPromptRegistry::default();
    let mut callbacks = RecordingCallbacks::default();
    let error = init_prompts(&mut registry, &all, None, &mut callbacks)
        .expect_err("langfuse has no initializer");
    assert!(matches!(
        &error,
        InitPromptsError::Initialize { prompt_id, .. } if prompt_id.as_str() == "second"
    ));
    assert_eq!(registry.in_memory_prompts.len(), 1);
    assert_eq!(callbacks.added.len(), 1);
}

#[test]
fn an_empty_list_registers_nothing() {
    let mut registry = InMemoryPromptRegistry::default();
    let mut callbacks = RecordingCallbacks::default();
    let registered =
        init_prompts(&mut registry, &[], None, &mut callbacks).expect("there is nothing to do");
    assert!(registered.is_empty());
    assert!(registry.in_memory_prompts.is_empty());
    assert!(registry.prompt_id_to_custom_prompt.is_empty());
    assert!(callbacks.added.is_empty());
}

#[test]
fn the_config_file_path_is_handed_on_without_being_read() {
    let all = entries(&[
        r#"{"prompt_id":"greeting","litellm_params":{"prompt_integration":"dotprompt"}}"#,
    ]);
    let mut registry = InMemoryPromptRegistry::default();
    let mut callbacks = RecordingCallbacks::default();
    let path = Path::new("/does/not/exist.yml");
    let registered = init_prompts(&mut registry, &all, Some(path), &mut callbacks)
        .expect("the path is not read");
    assert_eq!(registered.len(), 1);
}

#[test]
fn logs_the_prompt_list_once_after_the_loop() {
    let (messages, _capture) = capture_debug_messages();
    let all = entries(&[
        r#"{"prompt_id":"greeting","litellm_params":{"prompt_integration":"dotprompt"}}"#,
    ]);
    let mut registry = InMemoryPromptRegistry::default();
    let mut callbacks = RecordingCallbacks::default();
    init_prompts(&mut registry, &all, None, &mut callbacks).expect("the prompt registers");
    assert_eq!(prompt_list_logs(&messages), 1);
}

#[test]
fn an_empty_list_still_logs_the_prompt_list() {
    let (messages, _capture) = capture_debug_messages();
    let mut registry = InMemoryPromptRegistry::default();
    let mut callbacks = RecordingCallbacks::default();
    init_prompts(&mut registry, &[], None, &mut callbacks).expect("there is nothing to do");
    assert_eq!(prompt_list_logs(&messages), 1);
}

#[test]
fn a_failed_start_does_not_log_the_prompt_list() {
    let (messages, _capture) = capture_debug_messages();
    let all = entries(&[
        r#"{"prompt_id":"langfuse-only","litellm_params":{"prompt_integration":"langfuse"}}"#,
    ]);
    let mut registry = InMemoryPromptRegistry::default();
    let mut callbacks = RecordingCallbacks::default();
    init_prompts(&mut registry, &all, None, &mut callbacks)
        .expect_err("langfuse has no initializer");
    assert_eq!(prompt_list_logs(&messages), 0);
}
