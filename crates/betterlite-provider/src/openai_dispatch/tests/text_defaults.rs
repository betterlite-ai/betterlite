use secrecy::ExposeSecret as _;

use super::support;
use crate::openai_dispatch::call::{Credentials, TextCall};
use crate::openai_dispatch::fixtures::{FixtureLogger, FixtureTextSource, TextReply};
use crate::openai_dispatch::text;
use crate::operation::{ChatCompletion, OperationResult};

fn call(credentials: Credentials, optional_params: &str) -> TextCall {
    TextCall {
        model: String::from("gpt-3.5-turbo-instruct"),
        messages: support::messages(r#"[{"role":"user","content":"hi"}]"#),
        optional_params: support::optional_params(optional_params),
        credentials,
        headers: None,
        timeout: None,
        acompletion: false,
        text_completion: false,
    }
}

// The plan resolves the base and the key through B500 and the branch's own module constants.
pub fn check_plan() {
    let mut composition = support::composition();
    composition.globals = support::globals(
        Some("https://global.test/v1"),
        Some("sk-global"),
        None,
        Some("org-global"),
    );
    composition.text_defaults = support::text_defaults(r#"{"temperature":0.5,"top_p":0.9}"#);
    let plan = text::plan(
        &composition,
        &call(
            support::credentials(None, None, None),
            r#"{"temperature":null,"max_tokens":0}"#,
        ),
    )
    .expect("the text plan");
    assert_eq!(plan.api_base, "https://global.test/v1");
    assert_eq!(
        plan.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("sk-global")
    );
    assert_eq!(plan.api_type, "openai");
    assert_eq!(plan.api_version, None);
    // defaults merge by missing key only: an explicit null and zero stay as written
    assert_eq!(
        plan.optional_params.get("temperature"),
        Some(&serde_json::Value::Null)
    );
    assert_eq!(
        plan.optional_params.get("max_tokens"),
        Some(&serde_json::Value::from(0))
    );
    assert_eq!(
        plan.optional_params.get("top_p"),
        Some(&serde_json::Value::from(0.9))
    );

    // an explicit base and key win over the globals
    let explicit = text::plan(
        &composition,
        &call(
            support::credentials(Some("https://explicit.test/v1"), Some("sk-explicit"), None),
            r#"{"temperature":null,"max_tokens":0}"#,
        ),
    )
    .expect("the text plan");
    assert_eq!(explicit.api_base, "https://explicit.test/v1");
    assert_eq!(
        explicit.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("sk-explicit")
    );

    // with no explicit or global value the configured api_base stands, and the key is unset
    let plain = text::plan(
        &support::composition(),
        &call(support::credentials(None, None, None), "{}"),
    )
    .expect("the text plan");
    assert_eq!(plain.api_base, "https://api.openai.com/v1");
    assert!(plain.api_key.is_none());
}

// The transport receives the resolved base, key and timeout, and never an organization.
pub async fn check_transport_attempt() {
    let mut composition = support::composition();
    composition.globals = support::globals(
        Some("https://global.test/v1"),
        None,
        Some("sk-openai-global"),
        Some("org-global"),
    );
    composition.text_defaults = support::text_defaults(r#"{"temperature":0.5}"#);
    let text = FixtureTextSource::new(TextReply::new(200, support::TEXT_REPLY.as_bytes().to_vec()));
    let mut logger = FixtureLogger::default();
    let result = support::dispatch_text(
        &composition,
        call(
            support::credentials(None, None, Some("org-explicit")),
            r#"{"max_tokens":0}"#,
        ),
        &text,
        &mut logger,
    )
    .await
    .expect("the text dispatch");
    let attempts = text.attempts();
    assert_eq!(attempts.len(), 1);
    assert_eq!(
        attempts[0].base_url.as_deref(),
        Some("https://global.test/v1")
    );
    assert_eq!(
        attempts[0]
            .api_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("sk-openai-global")
    );
    // main.py assigns the SDK module global a truthy organization, and the handler's client never receives one
    assert_eq!(attempts[0].organization, None);
    assert_eq!(attempts[0].timeout, 6000.0);
    let bodies = text.bodies();
    assert_eq!(
        bodies[0].get("model"),
        Some(&serde_json::Value::String(String::from(
            "gpt-3.5-turbo-instruct"
        )))
    );
    // the absent temperature took the configured default, and the explicit zero stayed as written
    assert_eq!(
        bodies[0].get("temperature"),
        Some(&serde_json::Value::from(0.5))
    );
    assert_eq!(
        bodies[0].get("max_tokens"),
        Some(&serde_json::Value::from(0))
    );
    assert!(bodies[0].contains_key("prompt"));

    let OperationResult::Chat(ChatCompletion::OpenAi(reply)) = result else {
        panic!("a projected chat completion");
    };
    assert_eq!(reply.id, "cmpl-1");
    assert_eq!(reply.model, "gpt-3.5-turbo-instruct");
    assert_eq!(reply.choices[0].message.content.as_deref(), Some("hello"));
    assert_eq!(reply.usage.total_tokens, 2);
    // a synchronous text call is not an async one, so main.py logs no post_call
    assert!(logger.calls.is_empty());
}
