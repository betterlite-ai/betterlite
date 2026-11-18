use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::text_completion_transport::logging::{
    CompletionLogger as _, PostCallRecord, ResponseKind, SecretHeaders,
};
use secrecy::{ExposeSecret, SecretString};

use super::support;
use crate::openai_dispatch::call::{Composition, Credentials, TextCall};
use crate::openai_dispatch::fixtures::logger::FixtureLogger;
use crate::openai_dispatch::fixtures::text_transport::{FixtureTextSource, TextReply};
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

fn plan(
    composition: &Composition,
    credentials: Credentials,
    optional_params: &str,
) -> text::TextPlan {
    text::plan(composition, &call(credentials, optional_params)).expect("the text plan")
}

fn global_composition() -> Composition {
    let mut composition = support::composition();
    let identity = support::credentials(
        Some("https://global.test/v1"),
        Some("sk-global"),
        Some("org-global"),
    );
    composition.globals = support::globals(identity, None);
    composition.text_defaults = support::text_defaults(r#"{"temperature":0.5,"top_p":0.9}"#);
    composition
}

// The transport case moves the key to litellm.openai_key and keeps the rest of the global identity.
fn transport_composition() -> Composition {
    let mut composition = global_composition();
    composition.globals.api_key = None;
    composition.globals.openai_key = Some(SecretString::from("sk-openai-global"));
    composition.text_defaults = support::text_defaults(r#"{"temperature":0.5}"#);
    composition
}

async fn dispatched(
    composition: &Composition,
) -> (FixtureTextSource, FixtureLogger, OperationResult) {
    let text = FixtureTextSource::new(TextReply::new(200, support::TEXT_REPLY.as_bytes().to_vec()));
    let mut logger = FixtureLogger::default();
    let result = support::dispatch_text(
        composition,
        call(
            support::credentials(None, None, Some("org-explicit")),
            r#"{"max_tokens":0}"#,
        ),
        &text,
        &mut logger,
    )
    .await
    .expect("the text dispatch");
    (text, logger, result)
}

// The plan resolves the base and the key through B500 and the branch's own module constants.
pub(super) fn check_plan_resolution() {
    let composition = global_composition();
    let resolved = plan(
        &composition,
        support::credentials(None, None, None),
        r#"{"temperature":null,"max_tokens":0}"#,
    );
    assert_eq!(resolved.api_base, "https://global.test/v1");
    assert_eq!(
        resolved.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("sk-global")
    );
    assert_eq!(resolved.api_type, "openai");
    assert_eq!(resolved.api_version, None);
    // an explicit base and key win over the globals
    let explicit = plan(
        &composition,
        support::credentials(Some("https://explicit.test/v1"), Some("sk-explicit"), None),
        r#"{"temperature":null,"max_tokens":0}"#,
    );
    assert_eq!(explicit.api_base, "https://explicit.test/v1");
    assert_eq!(
        explicit.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("sk-explicit")
    );
}

// Defaults merge by missing key only: an explicit null and zero stay as written, the absent key takes the default.
pub(super) fn check_plan_defaults() {
    let resolved = plan(
        &global_composition(),
        support::credentials(None, None, None),
        r#"{"temperature":null,"max_tokens":0}"#,
    );
    let fields = support::fields_of(&resolved.optional_params);
    assert_eq!(fields.get("temperature"), Some(&JsonValue::Null));
    assert_eq!(fields.get("max_tokens"), Some(&support::number_value(0)));
    assert_eq!(fields.get("top_p"), Some(&support::real_value(0.9)));
}

// With no explicit or global value the configured api_base stands, and the key is unset.
pub(super) fn check_plan_default_base() {
    let plain = plan(
        &support::composition(),
        support::credentials(None, None, None),
        "{}",
    );
    assert_eq!(plain.api_base, "https://api.openai.com/v1");
    assert!(plain.api_key.is_none());
}

// The transport receives the resolved base, key and timeout, and never an organization.
pub(super) async fn check_transport_attempt() {
    let (text, logger, _result) = dispatched(&transport_composition()).await;
    let attempts = text.attempts();
    assert_eq!(attempts.len(), 1);
    // the transport opens every send through the logging port, so the pre_call is recorded
    assert_eq!(logger.pre_calls.len(), 1);
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
    assert!(attempts[0].timeout.total_cmp(&6000.0_f64).is_eq());
}

// The body carries the model, the configured default and the caller's explicit zero; the reply projects and logs nothing.
pub(super) async fn check_transport_body_and_reply() {
    let (text, logger, result) = dispatched(&transport_composition()).await;
    let bodies = text.bodies();
    let body = support::fields_of(&bodies[0]);
    assert_eq!(
        body.get("model"),
        Some(&support::text_value("gpt-3.5-turbo-instruct"))
    );
    // the absent temperature took the configured default, and the explicit zero stayed as written
    assert_eq!(body.get("temperature"), Some(&support::real_value(0.5)));
    assert_eq!(body.get("max_tokens"), Some(&support::number_value(0)));
    assert!(body.contains_key("prompt"));
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

// The completion port the transport may post through keeps the record it is handed.
pub(super) fn check_completion_post_call() {
    let mut logger = FixtureLogger::default();
    logger.post_call(PostCallRecord {
        api_key: None,
        original_response: serde_json::Value::Null,
        kind: ResponseKind::DumpedDict,
        headers: SecretHeaders::default(),
        api_base: Some(String::from("https://global.test/v1")),
    });
    assert_eq!(logger.post_calls.len(), 1);
    assert_eq!(
        logger.post_calls[0].api_base.as_deref(),
        Some("https://global.test/v1")
    );
}

pub(super) async fn check_all() {
    check_plan_resolution();
    check_plan_defaults();
    check_plan_default_base();
    check_transport_attempt().await;
    check_transport_body_and_reply().await;
    check_completion_post_call();
}
