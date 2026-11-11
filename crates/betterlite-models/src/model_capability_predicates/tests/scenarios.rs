use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use tracing::Level;
use tracing::subscriber::set_default;

use super::super::{CapabilityPredicates as _, CapabilityRequest};
use super::fixture::{Fixture, Session};
use crate::decisions::Json;

const SUPPORTS_RESOLUTION: &str = r#"{
    "gpt-4o": {"litellm_provider": "openai", "mode": "chat", "supports_function_calling": true},
    "openai/gpt-4o-flags": {"litellm_provider": "openai", "mode": "chat", "supports_function_calling": false},
    "gpt-4o-flags": {"litellm_provider": "openai", "mode": "chat", "supports_function_calling": true},
    "deepseek/deepseek-chat": {"litellm_provider": "deepseek", "mode": "chat", "supports_function_calling": 0},
    "deepseek-chat": {"litellm_provider": "deepseek", "mode": "chat", "supports_function_calling": true},
    "deepseek/string-model": {"litellm_provider": "deepseek", "mode": "chat", "supports_function_calling": "yes"},
    "string-model": {"litellm_provider": "deepseek", "mode": "chat", "supports_function_calling": true},
    "deepseek/prefixed-model": {"litellm_provider": "deepseek", "mode": "chat"},
    "prefixed-model": {"litellm_provider": "openai", "mode": "chat", "supports_function_calling": true},
    "github_copilot/gpt-4o": {"litellm_provider": "github_copilot", "mode": "chat", "supports_function_calling": true}
}"#;

const SUPPORTS_ERROR_FALLBACK: &str = r#"{
    "gpt-4o": {"litellm_provider": "openai", "mode": "chat", "supports_function_calling": true}
}"#;

const DECLARED_STRING_VALUE: &str = r#"{
    "reasoning-model": {"litellm_provider": "deepseek", "mode": "chat", "supports_reasoning": "stringy"},
    "empty-string-model": {"litellm_provider": "deepseek", "mode": "chat", "supports_custom": ""},
    "deepseek/bool-model": {"litellm_provider": "deepseek", "mode": "chat", "supports_custom": true},
    "bool-model": {"litellm_provider": "deepseek", "mode": "chat", "supports_custom": "bare-text"},
    "deepseek/number-model": {"litellm_provider": "deepseek", "mode": "chat", "supports_custom": 7},
    "number-model": {"litellm_provider": "deepseek", "mode": "chat", "supports_custom": "bare-number"},
    "deepseek/missing-model": {"litellm_provider": "deepseek", "mode": "chat"}
}"#;

const EXPLICIT_DISABLED: &str = r#"{
    "disabled-model": {"litellm_provider": "deepseek", "mode": "chat", "supports_vision": false},
    "enabled-model": {"litellm_provider": "deepseek", "mode": "chat", "supports_vision": true},
    "deepseek/bare-disabled": {"litellm_provider": "deepseek", "mode": "chat"},
    "bare-disabled": {"litellm_provider": "deepseek", "mode": "chat", "supports_vision": false},
    "deepseek/bare-enabled": {"litellm_provider": "deepseek", "mode": "chat"},
    "bare-enabled": {"litellm_provider": "deepseek", "mode": "chat", "supports_vision": true},
    "deepseek/silent-model": {"litellm_provider": "deepseek", "mode": "chat"},
    "github_copilot/gpt-4o": {"litellm_provider": "github_copilot", "mode": "chat", "supports_vision": false}
}"#;

const NAMED_CAPABILITY_WRAPPERS: &str = r#"{
    "system-model": {"litellm_provider": "deepseek", "mode": "chat", "supports_system_messages": true},
    "parallel-model": {"litellm_provider": "deepseek", "mode": "chat", "supports_parallel_function_calling": true},
    "function-model": {"litellm_provider": "deepseek", "mode": "chat", "supports_function_calling": true},
    "tool-choice-model": {"litellm_provider": "deepseek", "mode": "chat", "supports_tool_choice": true},
    "caching-model": {"litellm_provider": "deepseek", "mode": "chat", "supports_prompt_caching": true},
    "reasoning-model": {"litellm_provider": "deepseek", "mode": "chat", "supports_reasoning": true},
    "none-effort-model": {"litellm_provider": "deepseek", "mode": "chat", "supports_none_reasoning_effort": true},
    "mid-system-model": {"litellm_provider": "deepseek", "mode": "chat", "supports_mid_conversation_system": true},
    "structured-model": {"litellm_provider": "deepseek", "mode": "chat", "supports_native_structured_output": true},
    "silent-model": {"litellm_provider": "deepseek", "mode": "chat"}
}"#;

const NATIVE_STREAMING_DEFAULT: &str = r#"{
    "plain-model": {"litellm_provider": "deepseek", "mode": "chat"},
    "null-model": {"litellm_provider": "deepseek", "mode": "chat", "supports_native_streaming": null},
    "false-model": {"litellm_provider": "deepseek", "mode": "chat", "supports_native_streaming": false},
    "string-model": {"litellm_provider": "deepseek", "mode": "chat", "supports_native_streaming": "sometimes"},
    "true-model": {"litellm_provider": "deepseek", "mode": "chat", "supports_native_streaming": true}
}"#;

const RESPONSE_SCHEMA_PROVIDER_OVERRIDE: &str = r#"{
    "deepseek/schema-true": {"litellm_provider": "deepseek", "mode": "chat", "supports_response_schema": true},
    "deepseek/schema-false": {"litellm_provider": "deepseek", "mode": "chat", "supports_response_schema": false},
    "deepseek/schema-silent": {"litellm_provider": "deepseek", "mode": "chat"}
}"#;

pub(super) async fn supports_resolution() {
    let fixture = Fixture::new(SUPPORTS_RESOLUTION);
    let mut session = Session::new(&fixture);
    let key = "supports_function_calling";
    let inferred = CapabilityRequest {
        model: "gpt-4o",
        custom_llm_provider: None,
    };
    let inferred_answer = session.environment().supports_factory(&inferred, key).await;
    assert!(inferred_answer);
    let prefixed = CapabilityRequest {
        model: "openai/gpt-4o",
        custom_llm_provider: None,
    };
    let prefixed_answer = session.environment().supports_factory(&prefixed, key).await;
    assert!(prefixed_answer);
    let declared = CapabilityRequest {
        model: "github_copilot/gpt-4o",
        custom_llm_provider: None,
    };
    let declared_answer = session.environment().supports_factory(&declared, key).await;
    assert!(declared_answer);
    let blocked = CapabilityRequest {
        model: "openai/gpt-4o-flags",
        custom_llm_provider: None,
    };
    let blocked_answer = session.environment().supports_factory(&blocked, key).await;
    assert!(!blocked_answer);
    resolution_blocks_and_falls_back(&mut session, key).await;
}

async fn resolution_blocks_and_falls_back(session: &mut Session<'_>, key: &str) {
    let provider = Some("deepseek");
    let zeroed = CapabilityRequest {
        model: "deepseek/deepseek-chat",
        custom_llm_provider: provider,
    };
    assert!(!session.environment().supports_factory(&zeroed, key).await);
    let text = CapabilityRequest {
        model: "deepseek/string-model",
        custom_llm_provider: provider,
    };
    assert!(!session.environment().supports_factory(&text, key).await);
    let bare = CapabilityRequest {
        model: "deepseek/prefixed-model",
        custom_llm_provider: provider,
    };
    assert!(session.environment().supports_factory(&bare, key).await);
    let supported = CapabilityRequest {
        model: "fireworks_ai/mystery",
        custom_llm_provider: Some("fireworks_ai"),
    };
    assert!(
        session
            .environment()
            .supports_factory(&supported, key)
            .await
    );
    let silent = CapabilityRequest {
        model: "xai/mystery-two",
        custom_llm_provider: Some("xai"),
    };
    assert!(!session.environment().supports_factory(&silent, key).await);
    let unknown = CapabilityRequest {
        model: "nothing-here",
        custom_llm_provider: None,
    };
    assert!(!session.environment().supports_factory(&unknown, key).await);
}

pub(super) async fn supports_error_fallback() {
    let fixture = Fixture::new(SUPPORTS_ERROR_FALLBACK);
    let mut session = Session::new(&fixture);
    unresolved_failures_log_and_continue(&mut session).await;
    unmatched_failures_log_and_continue(&mut session).await;
}

async fn unresolved_failures_log_and_continue(session: &mut Session<'_>) {
    let unresolved = CapabilityRequest {
        model: "nothing-here",
        custom_llm_provider: None,
    };
    let (answer, log) = logged_supports(session, &unresolved, "supports_function_calling").await;
    assert!(!answer);
    assert!(
        log.contains(
            "Model not found or error in checking supports_function_calling support. \
             You passed model=nothing-here, custom_llm_provider=None. Error: "
        ),
        "{log}"
    );
}

async fn unmatched_failures_log_and_continue(session: &mut Session<'_>) {
    let unmatched = CapabilityRequest {
        model: "fireworks_ai/nothing-known",
        custom_llm_provider: Some("fireworks_ai"),
    };
    let (answer, log) = logged_supports(session, &unmatched, "supports_function_calling").await;
    assert!(answer, "{log}");
    assert!(
        log.contains("You passed model=nothing-known, custom_llm_provider=fireworks_ai. Error: "),
        "{log}"
    );
    let caching = session
        .environment()
        .supports_factory(&unmatched, "supports_prompt_caching")
        .await;
    assert!(caching);
    let stateless = CapabilityRequest {
        model: "openai/missing-model",
        custom_llm_provider: Some("openai"),
    };
    let stateless_answer = session
        .environment()
        .supports_factory(&stateless, "supports_function_calling")
        .await;
    assert!(!stateless_answer);
}

pub(super) async fn declared_string_value() {
    let fixture = Fixture::new(DECLARED_STRING_VALUE);
    let mut session = Session::new(&fixture);
    declared_values(&mut session).await;
    declared_number_values(&mut session).await;
    let provider = Some("deepseek");
    let missing = CapabilityRequest {
        model: "deepseek/missing-model",
        custom_llm_provider: provider,
    };
    let missing_value = session
        .environment()
        .declared_value_factory(&missing, "supports_custom")
        .await;
    assert_eq!(missing_value, None);
    let unknown = CapabilityRequest {
        model: "nothing-here",
        custom_llm_provider: None,
    };
    let unknown_value = session
        .environment()
        .declared_value_factory(&unknown, "supports_custom")
        .await;
    assert_eq!(unknown_value, None);
}

async fn declared_values(session: &mut Session<'_>) {
    let provider = Some("deepseek");
    let declared = CapabilityRequest {
        model: "reasoning-model",
        custom_llm_provider: provider,
    };
    let declared_value = session
        .environment()
        .declared_value_factory(&declared, "supports_reasoning")
        .await;
    assert_eq!(declared_value.as_deref(), Some("stringy"));
    let empty = CapabilityRequest {
        model: "empty-string-model",
        custom_llm_provider: provider,
    };
    let empty_value = session
        .environment()
        .declared_value_factory(&empty, "supports_custom")
        .await;
    assert_eq!(empty_value.as_deref(), Some(""));
    let boolean = CapabilityRequest {
        model: "deepseek/bool-model",
        custom_llm_provider: provider,
    };
    let boolean_value = session
        .environment()
        .declared_value_factory(&boolean, "supports_custom")
        .await;
    assert_eq!(boolean_value.as_deref(), Some("bare-text"));
}

async fn declared_number_values(session: &mut Session<'_>) {
    let number = CapabilityRequest {
        model: "deepseek/number-model",
        custom_llm_provider: Some("deepseek"),
    };
    let number_value = session
        .environment()
        .declared_value_factory(&number, "supports_custom")
        .await;
    assert_eq!(number_value.as_deref(), Some("bare-number"));
}

pub(super) async fn explicit_disabled() {
    let fixture = Fixture::new(EXPLICIT_DISABLED);
    let mut session = Session::new(&fixture);
    let provider = Some("deepseek");
    let disabled = CapabilityRequest {
        model: "disabled-model",
        custom_llm_provider: provider,
    };
    let disabled_answer = session
        .environment()
        .is_explicitly_disabled_factory(&disabled, "supports_vision")
        .await;
    assert!(disabled_answer);
    disabled_keeps_absent_models_eligible(&mut session).await;
}

async fn disabled_keeps_absent_models_eligible(session: &mut Session<'_>) {
    let provider = Some("deepseek");
    let enabled = CapabilityRequest {
        model: "enabled-model",
        custom_llm_provider: provider,
    };
    let enabled_answer = session
        .environment()
        .is_explicitly_disabled_factory(&enabled, "supports_vision")
        .await;
    assert!(!enabled_answer);
    let bare_disabled = CapabilityRequest {
        model: "deepseek/bare-disabled",
        custom_llm_provider: provider,
    };
    let bare_disabled_answer = session
        .environment()
        .is_explicitly_disabled_factory(&bare_disabled, "supports_vision")
        .await;
    assert!(bare_disabled_answer);
    let bare_enabled = CapabilityRequest {
        model: "deepseek/bare-enabled",
        custom_llm_provider: provider,
    };
    let bare_enabled_answer = session
        .environment()
        .is_explicitly_disabled_factory(&bare_enabled, "supports_vision")
        .await;
    assert!(!bare_enabled_answer);
    disabled_keeps_silent_models_eligible(session).await;
}

async fn disabled_keeps_silent_models_eligible(session: &mut Session<'_>) {
    let provider = Some("deepseek");
    let silent = CapabilityRequest {
        model: "deepseek/silent-model",
        custom_llm_provider: provider,
    };
    let silent_answer = session
        .environment()
        .is_explicitly_disabled_factory(&silent, "supports_vision")
        .await;
    assert!(!silent_answer);
    let declared = CapabilityRequest {
        model: "github_copilot/gpt-4o",
        custom_llm_provider: None,
    };
    let declared_answer = session
        .environment()
        .is_explicitly_disabled_factory(&declared, "supports_vision")
        .await;
    assert!(declared_answer);
    let unknown = CapabilityRequest {
        model: "nothing-here",
        custom_llm_provider: None,
    };
    let unknown_answer = session
        .environment()
        .is_explicitly_disabled_factory(&unknown, "supports_vision")
        .await;
    assert!(!unknown_answer);
}

pub(super) async fn named_capability_wrappers() {
    let fixture = Fixture::new(NAMED_CAPABILITY_WRAPPERS);
    let mut session = Session::new(&fixture);
    named_message_wrappers(&mut session).await;
    named_reasoning_wrappers(&mut session).await;
}

async fn named_message_wrappers(session: &mut Session<'_>) {
    let provider = Some("deepseek");
    let system = CapabilityRequest {
        model: "system-model",
        custom_llm_provider: provider,
    };
    assert!(
        session
            .environment()
            .supports_system_messages(&system)
            .await
    );
    let parallel = CapabilityRequest {
        model: "parallel-model",
        custom_llm_provider: provider,
    };
    assert!(
        session
            .environment()
            .supports_parallel_function_calling(&parallel)
            .await
    );
    let function = CapabilityRequest {
        model: "function-model",
        custom_llm_provider: provider,
    };
    assert!(
        session
            .environment()
            .supports_function_calling(&function)
            .await
    );
    named_choice_wrappers(session).await;
}

async fn named_choice_wrappers(session: &mut Session<'_>) {
    let provider = Some("deepseek");
    let tool_choice = CapabilityRequest {
        model: "tool-choice-model",
        custom_llm_provider: provider,
    };
    assert!(
        session
            .environment()
            .supports_tool_choice(&tool_choice)
            .await
    );
    let caching = CapabilityRequest {
        model: "caching-model",
        custom_llm_provider: provider,
    };
    assert!(
        session
            .environment()
            .supports_prompt_caching(&caching)
            .await
    );
    let reasoning = CapabilityRequest {
        model: "reasoning-model",
        custom_llm_provider: provider,
    };
    assert!(session.environment().supports_reasoning(&reasoning).await);
}

async fn named_reasoning_wrappers(session: &mut Session<'_>) {
    let provider = Some("deepseek");
    let effort = CapabilityRequest {
        model: "none-effort-model",
        custom_llm_provider: provider,
    };
    assert!(
        session
            .environment()
            .supports_none_reasoning_effort(&effort)
            .await
    );
    let mid_system = CapabilityRequest {
        model: "mid-system-model",
        custom_llm_provider: provider,
    };
    assert!(
        session
            .environment()
            .supports_mid_conversation_system(&mid_system)
            .await
    );
    let structured = CapabilityRequest {
        model: "structured-model",
        custom_llm_provider: provider,
    };
    assert!(
        session
            .environment()
            .supports_native_structured_output(&structured)
            .await
    );
    silent_system_messages(session).await;
}

async fn silent_system_messages(session: &mut Session<'_>) {
    let silent = CapabilityRequest {
        model: "silent-model",
        custom_llm_provider: Some("deepseek"),
    };
    let silent_answer = session
        .environment()
        .supports_system_messages(&silent)
        .await;
    assert!(!silent_answer);
}

pub(super) async fn native_streaming_default() {
    let fixture = Fixture::new(NATIVE_STREAMING_DEFAULT);
    let mut session = Session::new(&fixture);
    streaming_defaults(&mut session).await;
    streaming_passthrough(&mut session).await;
}

async fn streaming_defaults(session: &mut Session<'_>) {
    let provider = Some("deepseek");
    let plain = CapabilityRequest {
        model: "plain-model",
        custom_llm_provider: provider,
    };
    let plain_value = session
        .environment()
        .supports_native_streaming(&plain)
        .await;
    assert_eq!(plain_value, Json::Bool(true));
    let nulled = CapabilityRequest {
        model: "null-model",
        custom_llm_provider: provider,
    };
    let nulled_value = session
        .environment()
        .supports_native_streaming(&nulled)
        .await;
    assert_eq!(nulled_value, Json::Bool(true));
    let refused = CapabilityRequest {
        model: "false-model",
        custom_llm_provider: provider,
    };
    let refused_value = session
        .environment()
        .supports_native_streaming(&refused)
        .await;
    assert_eq!(refused_value, Json::Bool(false));
    let unknown = CapabilityRequest {
        model: "nothing-here",
        custom_llm_provider: None,
    };
    let unknown_value = session
        .environment()
        .supports_native_streaming(&unknown)
        .await;
    assert_eq!(unknown_value, Json::Bool(false));
}

async fn streaming_passthrough(session: &mut Session<'_>) {
    let provider = Some("deepseek");
    let passthrough = CapabilityRequest {
        model: "string-model",
        custom_llm_provider: provider,
    };
    let passthrough_value = session
        .environment()
        .supports_native_streaming(&passthrough)
        .await;
    assert_eq!(passthrough_value, Json::Text("sometimes".to_owned()));
    let accepted = CapabilityRequest {
        model: "true-model",
        custom_llm_provider: provider,
    };
    let accepted_value = session
        .environment()
        .supports_native_streaming(&accepted)
        .await;
    assert_eq!(accepted_value, Json::Bool(true));
}

pub(super) async fn response_schema_provider_override() {
    let fixture = Fixture::new(RESPONSE_SCHEMA_PROVIDER_OVERRIDE);
    let mut session = Session::new(&fixture);
    response_schema_overrides(&mut session).await;
    response_schema_map_flags(&mut session).await;
}

async fn response_schema_overrides(session: &mut Session<'_>) {
    let predibase = CapabilityRequest {
        model: "whatever",
        custom_llm_provider: Some("predibase"),
    };
    assert!(
        session
            .environment()
            .supports_response_schema(&predibase)
            .await
    );
    let fireworks = CapabilityRequest {
        model: "whatever",
        custom_llm_provider: Some("fireworks_ai"),
    };
    assert!(
        session
            .environment()
            .supports_response_schema(&fireworks)
            .await
    );
    let studio = CapabilityRequest {
        model: "whatever",
        custom_llm_provider: Some("lm_studio"),
    };
    assert!(
        session
            .environment()
            .supports_response_schema(&studio)
            .await
    );
    response_schema_remaining_overrides(session).await;
}

async fn response_schema_remaining_overrides(session: &mut Session<'_>) {
    let nebius = CapabilityRequest {
        model: "whatever",
        custom_llm_provider: Some("nebius"),
    };
    assert!(
        session
            .environment()
            .supports_response_schema(&nebius)
            .await
    );
    let databricks = CapabilityRequest {
        model: "whatever",
        custom_llm_provider: Some("databricks"),
    };
    assert!(
        session
            .environment()
            .supports_response_schema(&databricks)
            .await
    );
}

async fn response_schema_map_flags(session: &mut Session<'_>) {
    let provider = Some("deepseek");
    let flagged = CapabilityRequest {
        model: "deepseek/schema-true",
        custom_llm_provider: provider,
    };
    assert!(
        session
            .environment()
            .supports_response_schema(&flagged)
            .await
    );
    let refused = CapabilityRequest {
        model: "deepseek/schema-false",
        custom_llm_provider: provider,
    };
    assert!(
        !session
            .environment()
            .supports_response_schema(&refused)
            .await
    );
    response_schema_silent_models(session).await;
}

async fn response_schema_silent_models(session: &mut Session<'_>) {
    let silent = CapabilityRequest {
        model: "deepseek/schema-silent",
        custom_llm_provider: Some("deepseek"),
    };
    assert!(
        !session
            .environment()
            .supports_response_schema(&silent)
            .await
    );
    let unknown = CapabilityRequest {
        model: "nothing-here",
        custom_llm_provider: None,
    };
    assert!(
        !session
            .environment()
            .supports_response_schema(&unknown)
            .await
    );
}

async fn logged_supports(
    session: &mut Session<'_>,
    request: &CapabilityRequest<'_>,
    key: &str,
) -> (bool, String) {
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let writer = LogCapture(Arc::clone(&bytes));
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_target(false)
        .with_max_level(Level::DEBUG)
        .with_writer(move || writer.clone())
        .finish();
    let unknown_answer = {
        let _guard = set_default(subscriber);
        session.environment().supports_factory(request, key).await
    };
    let log = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
    (unknown_answer, log)
}

#[derive(Clone)]
struct LogCapture(Arc<Mutex<Vec<u8>>>);

impl io::Write for LogCapture {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
