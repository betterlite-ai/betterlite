use std::future::Future;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use tracing::Level;
use tracing::subscriber::set_default;

use super::super::{CapabilityPredicates as _, CapabilityRequest, NamedCapability};
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
    "github_copilot/gpt-4o": {"litellm_provider": "github_copilot", "mode": "chat", "supports_function_calling": true},
    "fireworks_ai/vision-model": {"litellm_provider": "openai", "mode": "chat", "supports_vision": true},
    "fireworks_ai/reason-model": {"litellm_provider": "deepseek", "mode": "chat", "supports_reasoning": true},
    "fireworks_ai/pdf-model": {"litellm_provider": "openai", "mode": "chat", "supports_pdf_input": true},
    "fireworks_ai/listed-model": {"litellm_provider": "fireworks_ai", "mode": "chat"}
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
    "deepseek/own-text-model": {"litellm_provider": "deepseek", "mode": "chat", "supports_custom": "own-text"},
    "own-text-model": {"litellm_provider": "deepseek", "mode": "chat", "supports_custom": 5},
    "deepseek/missing-model": {"litellm_provider": "deepseek", "mode": "chat"}
}"#;

const EXPLICIT_DISABLED: &str = r#"{
    "disabled-model": {"litellm_provider": "deepseek", "mode": "chat", "supports_vision": false},
    "enabled-model": {"litellm_provider": "deepseek", "mode": "chat", "supports_vision": true},
    "deepseek/bare-disabled": {"litellm_provider": "deepseek", "mode": "chat"},
    "bare-disabled": {"litellm_provider": "deepseek", "mode": "chat", "supports_vision": false},
    "deepseek/bare-enabled": {"litellm_provider": "deepseek", "mode": "chat"},
    "bare-enabled": {"litellm_provider": "deepseek", "mode": "chat", "supports_vision": true},
    "deepseek/zero-model": {"litellm_provider": "deepseek", "mode": "chat", "supports_vision": 0},
    "zero-model": {"litellm_provider": "deepseek", "mode": "chat", "supports_vision": false},
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

#[derive(Clone, Copy)]
enum Reader {
    Supports(&'static str),
    Declared(&'static str),
    Disabled(&'static str),
    Streaming,
    ResponseSchema,
}

// A plain struct, because a Text variant next to a bool one trips clippy's variant size lint.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Expected {
    flag: Option<bool>,
    text: Option<&'static str>,
}

impl Expected {
    const fn flag(value: bool) -> Self {
        Self {
            flag: Some(value),
            text: None,
        }
    }

    const fn text(value: &'static str) -> Self {
        Self {
            flag: None,
            text: Some(value),
        }
    }

    const fn nothing() -> Self {
        Self {
            flag: None,
            text: None,
        }
    }

    fn json(self) -> Json {
        match (self.flag, self.text) {
            (Some(flag), _) => Json::Bool(flag),
            (None, Some(text)) => Json::Text(text.to_owned()),
            (None, None) => Json::Null,
        }
    }
}

struct Scenario {
    fixture: &'static str,
    reader: Reader,
    steps: &'static [(&'static str, Option<&'static str>, Expected)],
}

struct NamedScenario {
    fixture: &'static str,
    steps: &'static [(NamedCapability, &'static str, Option<&'static str>, bool)],
}

async fn check(scenario: &Scenario) {
    let fixture = Fixture::new(scenario.fixture);
    let mut session = Session::new(&fixture);
    for (model, provider, expected) in scenario.steps {
        let request = CapabilityRequest {
            model,
            custom_llm_provider: *provider,
        };
        let answer = match scenario.reader {
            Reader::Supports(key) => {
                Json::Bool(session.environment().supports_factory(&request, key).await)
            }
            Reader::Declared(key) => session
                .environment()
                .declared_value_factory(&request, key)
                .await
                .map_or(Json::Null, Json::Text),
            Reader::Disabled(key) => Json::Bool(
                session
                    .environment()
                    .is_explicitly_disabled_factory(&request, key)
                    .await,
            ),
            Reader::Streaming => {
                session
                    .environment()
                    .supports_native_streaming(&request)
                    .await
            }
            Reader::ResponseSchema => Json::Bool(
                session
                    .environment()
                    .supports_response_schema(&request)
                    .await,
            ),
        };
        assert_eq!(answer, expected.json(), "{model}");
    }
}

async fn check_named(scenario: &NamedScenario) {
    let fixture = Fixture::new(scenario.fixture);
    let mut session = Session::new(&fixture);
    for (capability, model, provider, expected) in scenario.steps {
        let request = CapabilityRequest {
            model,
            custom_llm_provider: *provider,
        };
        let answer = session
            .environment()
            .supports_factory(&request, capability.key())
            .await;
        assert_eq!(answer, *expected, "{model}");
    }
}

const SUPPORTS_RESOLUTION_STEPS: &[(&str, Option<&str>, Expected)] = &[
    ("gpt-4o", None, Expected::flag(true)),
    ("openai/gpt-4o", None, Expected::flag(true)),
    ("github_copilot/gpt-4o", None, Expected::flag(true)),
    ("openai/gpt-4o-flags", None, Expected::flag(false)),
    (
        "deepseek/deepseek-chat",
        Some("deepseek"),
        Expected::flag(false),
    ),
    (
        "deepseek/string-model",
        Some("deepseek"),
        Expected::flag(false),
    ),
    (
        "deepseek/prefixed-model",
        Some("deepseek"),
        Expected::flag(true),
    ),
    (
        "fireworks_ai/mystery",
        Some("fireworks_ai"),
        Expected::flag(true),
    ),
    ("xai/mystery-two", Some("xai"), Expected::flag(false)),
    ("nothing-here", None, Expected::flag(false)),
];

const DECLARED_CUSTOM_STEPS: &[(&str, Option<&str>, Expected)] = &[
    ("empty-string-model", Some("deepseek"), Expected::text("")),
    (
        "deepseek/bool-model",
        Some("deepseek"),
        Expected::text("bare-text"),
    ),
    (
        "deepseek/number-model",
        Some("deepseek"),
        Expected::text("bare-number"),
    ),
    (
        "deepseek/own-text-model",
        Some("deepseek"),
        Expected::text("own-text"),
    ),
    (
        "deepseek/missing-model",
        Some("deepseek"),
        Expected::nothing(),
    ),
    ("nothing-here", None, Expected::nothing()),
];

const EXPLICIT_DISABLED_STEPS: &[(&str, Option<&str>, Expected)] = &[
    ("disabled-model", Some("deepseek"), Expected::flag(true)),
    ("enabled-model", Some("deepseek"), Expected::flag(false)),
    (
        "deepseek/bare-disabled",
        Some("deepseek"),
        Expected::flag(true),
    ),
    (
        "deepseek/bare-enabled",
        Some("deepseek"),
        Expected::flag(false),
    ),
    (
        "deepseek/silent-model",
        Some("deepseek"),
        Expected::flag(false),
    ),
    (
        "deepseek/zero-model",
        Some("deepseek"),
        Expected::flag(false),
    ),
    ("github_copilot/gpt-4o", None, Expected::flag(true)),
    ("nothing-here", None, Expected::flag(false)),
];

const NAMED_CAPABILITY_STEPS: &[(NamedCapability, &str, Option<&str>, bool)] = &[
    (
        NamedCapability::SystemMessages,
        "system-model",
        Some("deepseek"),
        true,
    ),
    (
        NamedCapability::ParallelCalls,
        "parallel-model",
        Some("deepseek"),
        true,
    ),
    (
        NamedCapability::FunctionCalling,
        "function-model",
        Some("deepseek"),
        true,
    ),
    (
        NamedCapability::ToolChoice,
        "tool-choice-model",
        Some("deepseek"),
        true,
    ),
    (
        NamedCapability::PromptCaching,
        "caching-model",
        Some("deepseek"),
        true,
    ),
    (
        NamedCapability::Reasoning,
        "reasoning-model",
        Some("deepseek"),
        true,
    ),
    (
        NamedCapability::NoneReasoningEffort,
        "none-effort-model",
        Some("deepseek"),
        true,
    ),
    (
        NamedCapability::MidConversationSystem,
        "mid-system-model",
        Some("deepseek"),
        true,
    ),
    (
        NamedCapability::NativeStructuredOutput,
        "structured-model",
        Some("deepseek"),
        true,
    ),
    (
        NamedCapability::SystemMessages,
        "silent-model",
        Some("deepseek"),
        false,
    ),
];

const NATIVE_STREAMING_STEPS: &[(&str, Option<&str>, Expected)] = &[
    ("plain-model", Some("deepseek"), Expected::flag(true)),
    ("null-model", Some("deepseek"), Expected::flag(true)),
    ("false-model", Some("deepseek"), Expected::flag(false)),
    (
        "string-model",
        Some("deepseek"),
        Expected::text("sometimes"),
    ),
    ("true-model", Some("deepseek"), Expected::flag(true)),
    ("nothing-here", None, Expected::flag(false)),
];

const RESPONSE_SCHEMA_STEPS: &[(&str, Option<&str>, Expected)] = &[
    ("whatever", Some("predibase"), Expected::flag(true)),
    ("whatever", Some("fireworks_ai"), Expected::flag(true)),
    ("whatever", Some("lm_studio"), Expected::flag(true)),
    ("whatever", Some("nebius"), Expected::flag(true)),
    ("whatever", Some("databricks"), Expected::flag(true)),
    (
        "deepseek/schema-true",
        Some("deepseek"),
        Expected::flag(true),
    ),
    (
        "deepseek/schema-false",
        Some("deepseek"),
        Expected::flag(false),
    ),
    (
        "deepseek/schema-silent",
        Some("deepseek"),
        Expected::flag(false),
    ),
    ("nothing-here", None, Expected::flag(false)),
];

const FUNCTION_CALLING_KEY: &str = "supports_function_calling";
const PROMPT_CACHING_KEY: &str = "supports_prompt_caching";
const REASONING_KEY: &str = "supports_reasoning";
const CUSTOM_KEY: &str = "supports_custom";
const VISION_KEY: &str = "supports_vision";
const PDF_INPUT_KEY: &str = "supports_pdf_input";
const WEB_SEARCH_KEY: &str = "supports_web_search";

const REASONING_DECLARATION_STEPS: &[(&str, Option<&str>, Expected)] = &[(
    "reasoning-model",
    Some("deepseek"),
    Expected::text("stringy"),
)];

const SUPPORTS_RESOLUTION_SCENARIO: Scenario = Scenario {
    fixture: SUPPORTS_RESOLUTION,
    reader: Reader::Supports(FUNCTION_CALLING_KEY),
    steps: SUPPORTS_RESOLUTION_STEPS,
};

// These declarations carry another provider's name, so only the provider info answers them.
const VISION_PROVIDER_STEPS: &[(&str, Option<&str>, Expected)] =
    &[("vision-model", Some("fireworks_ai"), Expected::flag(true))];

const VISION_PROVIDER_SCENARIO: Scenario = Scenario {
    fixture: SUPPORTS_RESOLUTION,
    reader: Reader::Supports(VISION_KEY),
    steps: VISION_PROVIDER_STEPS,
};

const REASONING_PROVIDER_STEPS: &[(&str, Option<&str>, Expected)] =
    &[("reason-model", Some("fireworks_ai"), Expected::flag(true))];

const REASONING_PROVIDER_SCENARIO: Scenario = Scenario {
    fixture: SUPPORTS_RESOLUTION,
    reader: Reader::Supports(REASONING_KEY),
    steps: REASONING_PROVIDER_STEPS,
};

const PDF_PROVIDER_STEPS: &[(&str, Option<&str>, Expected)] =
    &[("pdf-model", Some("fireworks_ai"), Expected::flag(true))];

const PDF_PROVIDER_SCENARIO: Scenario = Scenario {
    fixture: SUPPORTS_RESOLUTION,
    reader: Reader::Supports(PDF_INPUT_KEY),
    steps: PDF_PROVIDER_STEPS,
};

const WEB_SEARCH_PROVIDER_STEPS: &[(&str, Option<&str>, Expected)] =
    &[("xai/mystery-two", Some("xai"), Expected::flag(true))];

const WEB_SEARCH_PROVIDER_SCENARIO: Scenario = Scenario {
    fixture: SUPPORTS_RESOLUTION,
    reader: Reader::Supports(WEB_SEARCH_KEY),
    steps: WEB_SEARCH_PROVIDER_STEPS,
};

// The mapped entry omits the key and has no bare twin, so only the provider info answers this.
const PROMPT_CACHING_PROVIDER_STEPS: &[(&str, Option<&str>, Expected)] =
    &[("listed-model", Some("fireworks_ai"), Expected::flag(true))];

const PROMPT_CACHING_PROVIDER_SCENARIO: Scenario = Scenario {
    fixture: SUPPORTS_RESOLUTION,
    reader: Reader::Supports(PROMPT_CACHING_KEY),
    steps: PROMPT_CACHING_PROVIDER_STEPS,
};

const REASONING_DECLARATION_SCENARIO: Scenario = Scenario {
    fixture: DECLARED_STRING_VALUE,
    reader: Reader::Declared(REASONING_KEY),
    steps: REASONING_DECLARATION_STEPS,
};

const DECLARED_CUSTOM_SCENARIO: Scenario = Scenario {
    fixture: DECLARED_STRING_VALUE,
    reader: Reader::Declared(CUSTOM_KEY),
    steps: DECLARED_CUSTOM_STEPS,
};

const EXPLICIT_DISABLED_SCENARIO: Scenario = Scenario {
    fixture: EXPLICIT_DISABLED,
    reader: Reader::Disabled(VISION_KEY),
    steps: EXPLICIT_DISABLED_STEPS,
};

const NAMED_CAPABILITY_SCENARIO: NamedScenario = NamedScenario {
    fixture: NAMED_CAPABILITY_WRAPPERS,
    steps: NAMED_CAPABILITY_STEPS,
};

const NATIVE_STREAMING_SCENARIO: Scenario = Scenario {
    fixture: NATIVE_STREAMING_DEFAULT,
    reader: Reader::Streaming,
    steps: NATIVE_STREAMING_STEPS,
};

const RESPONSE_SCHEMA_SCENARIO: Scenario = Scenario {
    fixture: RESPONSE_SCHEMA_PROVIDER_OVERRIDE,
    reader: Reader::ResponseSchema,
    steps: RESPONSE_SCHEMA_STEPS,
};

pub(super) async fn supports_resolution() {
    check(&SUPPORTS_RESOLUTION_SCENARIO).await;
    check(&VISION_PROVIDER_SCENARIO).await;
    check(&REASONING_PROVIDER_SCENARIO).await;
    check(&PDF_PROVIDER_SCENARIO).await;
    check(&WEB_SEARCH_PROVIDER_SCENARIO).await;
    check(&PROMPT_CACHING_PROVIDER_SCENARIO).await;
}

pub(super) async fn declared_string_value() {
    check(&REASONING_DECLARATION_SCENARIO).await;
    check(&DECLARED_CUSTOM_SCENARIO).await;
    let fixture = Fixture::new(DECLARED_STRING_VALUE);
    let mut session = Session::new(&fixture);
    let prefixed = CapabilityRequest {
        model: "fireworks_ai/nothing-known",
        custom_llm_provider: Some("fireworks_ai"),
    };
    let (answer, log) = log_of(async {
        session
            .environment()
            .declared_value_factory(&prefixed, CUSTOM_KEY)
            .await
    })
    .await;
    assert_eq!(answer, None);
    assert!(
        log.contains(
            "Model not found or error in reading supports_custom. \
             You passed model=fireworks_ai/nothing-known, custom_llm_provider=fireworks_ai. Error: "
        ),
        "{log}"
    );
}

pub(super) async fn explicit_disabled() {
    check(&EXPLICIT_DISABLED_SCENARIO).await;
    let fixture = Fixture::new(EXPLICIT_DISABLED);
    let mut session = Session::new(&fixture);
    let unmapped = CapabilityRequest {
        model: "fireworks_ai/nothing-known",
        custom_llm_provider: Some("fireworks_ai"),
    };
    let (answer, log) = log_of(async {
        session
            .environment()
            .is_explicitly_disabled_factory(&unmapped, VISION_KEY)
            .await
    })
    .await;
    assert!(!answer);
    assert!(
        log.contains(
            "Model not found or error in checking supports_vision disabled state. \
             You passed model=nothing-known, custom_llm_provider=fireworks_ai. Error: "
        ),
        "{log}"
    );
}

pub(super) async fn named_capability_wrappers() {
    check_named(&NAMED_CAPABILITY_SCENARIO).await;
}

pub(super) async fn native_streaming_default() {
    check(&NATIVE_STREAMING_SCENARIO).await;
    let fixture = Fixture::new(NATIVE_STREAMING_DEFAULT);
    let mut session = Session::new(&fixture);
    let unmapped = CapabilityRequest {
        model: "fireworks_ai/nothing-known",
        custom_llm_provider: Some("fireworks_ai"),
    };
    let (answer, log) = log_of(async {
        session
            .environment()
            .supports_native_streaming(&unmapped)
            .await
    })
    .await;
    assert_eq!(answer, Json::Bool(false));
    assert!(
        log.contains(
            "Model not found or error in checking supports_native_streaming support. \
             You passed model=nothing-known, custom_llm_provider=fireworks_ai. Error: "
        ),
        "{log}"
    );
}

pub(super) async fn response_schema_provider_override() {
    check(&RESPONSE_SCHEMA_SCENARIO).await;
    let fixture = Fixture::new(RESPONSE_SCHEMA_PROVIDER_OVERRIDE);
    let mut session = Session::new(&fixture);
    let unresolved = CapabilityRequest {
        model: "nothing-here",
        custom_llm_provider: None,
    };
    let (answer, log) = log_of(async {
        session
            .environment()
            .supports_response_schema(&unresolved)
            .await
    })
    .await;
    assert!(!answer);
    assert!(
        log.contains(
            "Model not found or error in checking response schema support. \
             You passed model=nothing-here, custom_llm_provider=None. Error: "
        ),
        "{log}"
    );
}

pub(super) async fn supports_error_fallback() {
    let fixture = Fixture::new(SUPPORTS_ERROR_FALLBACK);
    let mut session = Session::new(&fixture);
    let unresolved = CapabilityRequest {
        model: "nothing-here",
        custom_llm_provider: None,
    };
    let (answer, log) = log_of(async {
        session
            .environment()
            .supports_factory(&unresolved, FUNCTION_CALLING_KEY)
            .await
    })
    .await;
    assert!(!answer);
    assert!(
        log.contains(
            "Model not found or error in checking supports_function_calling support. \
             You passed model=nothing-here, custom_llm_provider=None. Error: "
        ),
        "{log}"
    );
    unmatched_failure_falls_back(&mut session).await;
}

async fn unmatched_failure_falls_back(session: &mut Session<'_>) {
    let unmatched = CapabilityRequest {
        model: "fireworks_ai/nothing-known",
        custom_llm_provider: Some("fireworks_ai"),
    };
    let (supported, log) = log_of(async {
        session
            .environment()
            .supports_factory(&unmatched, FUNCTION_CALLING_KEY)
            .await
    })
    .await;
    assert!(supported, "{log}");
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
        .supports_factory(&stateless, FUNCTION_CALLING_KEY)
        .await;
    assert!(!stateless_answer);
}

// The subscriber must wrap only the reader call, so the capture holds that call's debug line.
async fn log_of<F, T>(call: F) -> (T, String)
where
    F: Future<Output = T>,
{
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let writer = LogCapture(Arc::clone(&bytes));
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_target(false)
        .with_max_level(Level::DEBUG)
        .with_writer(move || writer.clone())
        .finish();
    let answer = {
        let _guard = set_default(subscriber);
        call.await
    };
    let log = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
    (answer, log)
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
