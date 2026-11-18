use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::future::{Future, pending};
use std::path::PathBuf;
use std::process;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_openai::chat::{ChatMessage, ContentPart, MessageContent, Role};
use indexmap::IndexMap;
use serde_json::json;
use tokio::time::sleep;
use tokio_util::task::TaskTracker;

use crate::semantic_router::service::{
    AutoRouter, AutoRouterSettings, CallerContext, RouteDecision, RouteEmbedder,
};

// the two routes most tests score against; the utterances carry no threshold of their own
pub(crate) const TWO_ROUTES: &str = r#"{"routes": [
    {"name": "code", "utterances": ["write code"], "score_threshold": 0.5},
    {"name": "chat", "utterances": ["small talk"], "score_threshold": 0.5}
]}"#;

// one control character no test text contains, so a batch signature never collides
const BATCH_SEPARATOR: char = '\u{1f}';

#[derive(Debug, thiserror::Error)]
pub(crate) enum FakeError {
    #[error("the fake embedder has no vector for {text:?}")]
    Missing { text: String },
    #[error("the fake embedder was told to fail")]
    Injected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DocumentCall {
    pub(crate) model: String,
    pub(crate) texts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct QueryCall {
    pub(crate) model: String,
    pub(crate) text: String,
    pub(crate) raw_text: String,
    pub(crate) caller: CallerContext,
}

pub(crate) struct FakeEmbedder {
    vectors: BTreeMap<String, Vec<f64>>,
    documents: Mutex<Vec<DocumentCall>>,
    queries: Mutex<Vec<QueryCall>>,
    document_answers: BTreeMap<String, Vec<Vec<f64>>>,
    query_answers: BTreeMap<String, Vec<Vec<f64>>>,
    document_failures: AtomicUsize,
    query_failures: AtomicUsize,
    document_stalls: AtomicUsize,
    query_stalls: AtomicUsize,
    delay: Duration,
}

impl FakeEmbedder {
    pub(crate) fn new(vectors: &[(&str, Vec<f64>)]) -> Self {
        let mut found = BTreeMap::new();
        for (text, vector) in vectors {
            found.insert((*text).to_owned(), vector.clone());
        }
        Self {
            vectors: found,
            documents: Mutex::new(Vec::new()),
            queries: Mutex::new(Vec::new()),
            document_answers: BTreeMap::new(),
            query_answers: BTreeMap::new(),
            document_failures: AtomicUsize::new(0),
            query_failures: AtomicUsize::new(0),
            document_stalls: AtomicUsize::new(0),
            query_stalls: AtomicUsize::new(0),
            delay: Duration::ZERO,
        }
    }

    // the canned answer wins over the vectors, so a batch can break the count or the width contract
    #[must_use]
    pub(crate) fn documents_answer(mut self, texts: &[&str], vectors: Vec<Vec<f64>>) -> Self {
        self.document_answers
            .insert(batch_signature(texts), vectors);
        self
    }

    #[must_use]
    pub(crate) fn query_answers(mut self, text: &str, vectors: Vec<Vec<f64>>) -> Self {
        self.query_answers.insert(text.to_owned(), vectors);
        self
    }

    #[must_use]
    pub(crate) fn failing_documents(mut self, count: usize) -> Self {
        self.document_failures = AtomicUsize::new(count);
        self
    }

    #[must_use]
    pub(crate) fn failing_queries(mut self, count: usize) -> Self {
        self.query_failures = AtomicUsize::new(count);
        self
    }

    // a stalling call records its request and then never answers, so a caller's deadline is what ends it
    #[must_use]
    pub(crate) fn stalling_documents(mut self, count: usize) -> Self {
        self.document_stalls = AtomicUsize::new(count);
        self
    }

    #[must_use]
    pub(crate) fn stalling_queries(mut self, count: usize) -> Self {
        self.query_stalls = AtomicUsize::new(count);
        self
    }

    #[must_use]
    pub(crate) fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    pub(crate) fn document_calls(&self) -> Vec<DocumentCall> {
        self.documents
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn query_calls(&self) -> Vec<QueryCall> {
        self.queries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    // the probe is the first batch; the utterances are the second
    pub(crate) fn utterance_texts(&self) -> Vec<String> {
        self.document_calls()
            .get(1)
            .map_or_else(Vec::new, |call| call.texts.clone())
    }

    fn vector(&self, text: &str) -> Result<Vec<f64>, FakeError> {
        self.vectors
            .get(text)
            .cloned()
            .ok_or_else(|| FakeError::Missing {
                text: text.to_owned(),
            })
    }

    async fn wait_and_fail(&self, counter: &AtomicUsize) -> Result<(), FakeError> {
        sleep(self.delay).await;
        if take_failure(counter) {
            return Err(FakeError::Injected);
        }
        Ok(())
    }

    async fn document_vectors(
        &self,
        model: &str,
        texts: Vec<String>,
    ) -> Result<Vec<Vec<f64>>, FakeError> {
        record(
            &self.documents,
            DocumentCall {
                model: model.to_owned(),
                texts: texts.clone(),
            },
        );
        if take_failure(&self.document_stalls) {
            pending::<()>().await;
        }
        self.wait_and_fail(&self.document_failures).await?;
        let borrowed: Vec<&str> = texts.iter().map(String::as_str).collect();
        if let Some(vectors) = self.document_answers.get(&batch_signature(&borrowed)) {
            return Ok(vectors.clone());
        }
        let mut vectors = Vec::with_capacity(texts.len());
        for text in texts {
            vectors.push(self.vector(&text)?);
        }
        Ok(vectors)
    }

    async fn answer_query(
        &self,
        model: &str,
        text: String,
        raw_text: &str,
        caller: &CallerContext,
    ) -> Result<Vec<Vec<f64>>, FakeError> {
        record(
            &self.queries,
            QueryCall {
                model: model.to_owned(),
                text: text.clone(),
                raw_text: raw_text.to_owned(),
                caller: caller.clone(),
            },
        );
        if take_failure(&self.query_stalls) {
            pending::<()>().await;
        }
        self.wait_and_fail(&self.query_failures).await?;
        if let Some(vectors) = self.query_answers.get(&text) {
            return Ok(vectors.clone());
        }
        Ok(vec![self.vector(&text)?])
    }
}

fn batch_signature(texts: &[&str]) -> String {
    texts.join(&BATCH_SEPARATOR.to_string())
}

fn record<T>(calls: &Mutex<Vec<T>>, call: T) {
    calls
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(call);
}

impl RouteEmbedder for FakeEmbedder {
    type Error = FakeError;

    fn embed_documents(
        &self,
        model: &str,
        texts: Vec<String>,
    ) -> impl Future<Output = Result<Vec<Vec<f64>>, Self::Error>> + Send {
        self.document_vectors(model, texts)
    }

    fn embed_query(
        &self,
        model: &str,
        text: String,
        raw_text: &str,
        caller: &CallerContext,
    ) -> impl Future<Output = Result<Vec<Vec<f64>>, Self::Error>> + Send {
        self.answer_query(model, text, raw_text, caller)
    }
}

// a failed call consumes one injected failure only, so a retry can succeed
fn take_failure(counter: &AtomicUsize) -> bool {
    counter
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
            remaining.checked_sub(1)
        })
        .is_ok()
}

pub(crate) fn settings(config: &str) -> AutoRouterSettings {
    AutoRouterSettings {
        config_path: None,
        config_json: Some(config.to_owned()),
        default_model: "fallback".to_owned(),
        embedding_model: "fake-embed".to_owned(),
        max_input_chars: None,
    }
}

pub(crate) fn with_limit(mut settings: AutoRouterSettings, limit: i64) -> AutoRouterSettings {
    settings.max_input_chars = Some(limit);
    settings
}

pub(crate) fn router(config: &str, embedder: &Arc<FakeEmbedder>) -> AutoRouter<FakeEmbedder> {
    AutoRouter::new(settings(config), Arc::clone(embedder), TaskTracker::new())
        .expect("the route config parses")
}

pub(crate) fn caller() -> CallerContext {
    CallerContext::default()
}

pub(crate) fn decisions(route: Option<&str>, model: &str) -> RouteDecision {
    RouteDecision {
        route: route.map(str::to_owned),
        model: model.to_owned(),
    }
}

pub(crate) fn message(role: Role, content: Option<MessageContent>) -> ChatMessage {
    ChatMessage {
        role,
        content,
        name: None,
        tool_calls: None,
        tool_call_id: None,
        passthrough: IndexMap::new(),
    }
}

pub(crate) fn user_text(text: &str) -> ChatMessage {
    message(Role::User, Some(MessageContent::Text(text.to_owned())))
}

pub(crate) fn text_part(text: &str) -> ContentPart {
    ContentPart {
        kind: "text".to_owned(),
        fields: IndexMap::from([("text".to_owned(), json!(text))]),
    }
}

pub(crate) fn keyless_text_part() -> ContentPart {
    ContentPart {
        kind: "text".to_owned(),
        fields: IndexMap::new(),
    }
}

pub(crate) fn numeric_text_part() -> ContentPart {
    ContentPart {
        kind: "text".to_owned(),
        fields: IndexMap::from([("text".to_owned(), json!(5))]),
    }
}

pub(crate) fn image_part() -> ContentPart {
    ContentPart {
        kind: "image_url".to_owned(),
        fields: IndexMap::from([
            ("url".to_owned(), json!("https://example.test/i.png")),
            ("text".to_owned(), json!("not a text block")),
        ]),
    }
}

pub(crate) fn user_parts(parts: Vec<ContentPart>) -> ChatMessage {
    message(Role::User, Some(MessageContent::Parts(parts)))
}

pub(crate) fn temporary_directory(name: &str) -> PathBuf {
    let directory = env::temp_dir().join(format!("betterlite-b327-{name}-{}", process::id()));
    fs::create_dir_all(&directory).expect("the temporary directory is writable");
    directory
}
