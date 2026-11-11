use std::io::{self, Write};
use std::sync::{Arc, Mutex, PoisonError};

use tracing::subscriber::set_default;

use super::{
    DynamicCall, DynamicModelInfo, Fixture, Reply, ScriptedDynamic, raw, resolve, string_field,
};

pub(super) async fn dynamic_provider_metadata_case() {
    let fixture = fixture();
    authoritative_case(&fixture).await;
    fallback_case(&fixture).await;
}

fn fixture() -> Fixture {
    Fixture::new(
        r#"{
            "ollama/llama3.2": {"litellm_provider": "ollama", "mode": "chat", "input_cost_per_token": 9},
            "openai/gpt-4o": {"litellm_provider": "openai", "mode": "chat", "input_cost_per_token": 7}
        }"#,
    )
}

async fn authoritative_case(fixture: &Fixture) {
    let dynamic = ScriptedDynamic::new(vec![Reply::Value(raw(
        r#"{"key": "runtime", "max_tokens": 32}"#,
    ))]);
    let value = resolve(fixture, "llama3.2", Some("ollama"), &dynamic)
        .await
        .unwrap();
    assert_eq!(string_field(&value, "key"), "runtime");
    assert_eq!(
        dynamic.calls(),
        vec![DynamicCall {
            adapter: DynamicModelInfo::Ollama,
            model: "llama3.2".to_owned(),
            api_base: None,
            api_key: None,
        }]
    );

    let empty = ScriptedDynamic::new(vec![Reply::Value(raw("{}"))]);
    let projected = resolve(fixture, "llama3.2", Some("ollama"), &empty)
        .await
        .unwrap();
    assert!(projected.is_empty());

    let alias = ScriptedDynamic::new(vec![Reply::Value(raw(r#"{"key": "rewritten"}"#))]);
    let rewritten = resolve(fixture, "ada", Some("ollama"), &alias)
        .await
        .unwrap();
    assert_eq!(string_field(&rewritten, "key"), "rewritten");
    assert_eq!(alias.calls()[0].model, "azure/ada");
}

async fn fallback_case(fixture: &Fixture) {
    let scenarios = [
        (
            Reply::Error,
            "llama3.2",
            "ollama",
            "ollama/llama3.2",
            1_usize,
        ),
        (Reply::None, "llama3.2", "ollama", "ollama/llama3.2", 1),
        (
            Reply::Value(raw(r#"{"key": "ignored"}"#)),
            "openai/gpt-4o",
            "openai",
            "openai/gpt-4o",
            0,
        ),
    ];
    for (reply, model, provider, key, calls) in scenarios {
        let dynamic = ScriptedDynamic::new(vec![reply]);
        let value = resolve(fixture, model, Some(provider), &dynamic)
            .await
            .unwrap();
        assert_eq!(string_field(&value, "key"), key);
        assert_eq!(dynamic.calls().len(), calls);
    }
}

#[derive(Clone)]
struct LogCapture(Arc<Mutex<Vec<u8>>>);

impl Write for LogCapture {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .flush()
    }
}

#[tokio::test]
async fn dynamic_failures_warn_before_falling_back() {
    let fixture = fixture();
    let dynamic = ScriptedDynamic::new(vec![Reply::Error]);
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let writer = LogCapture(Arc::clone(&bytes));
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_target(false)
        .with_writer(move || writer.clone())
        .finish();
    let answer = {
        let _guard = set_default(subscriber);
        resolve(&fixture, "llama3.2", Some("ollama"), &dynamic).await
    };
    let log = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
    assert!(
        log.contains(
            "Could not get dynamic model info for model=llama3.2, provider=ollama; falling back to the static cost map"
        ),
        "{log}"
    );
    assert_eq!(string_field(&answer.unwrap(), "key"), "ollama/llama3.2");
}
