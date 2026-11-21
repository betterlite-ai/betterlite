use std::collections::BTreeMap;
use std::future::{Future, pending};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_config::sections::complexity_turn_overrides::{KeywordMatching, KeywordTierRule};
use betterlite_models::decisions::Json;
use betterlite_openai::chat::{ChatMessage, ContentPart, MessageContent, Role};
use indexmap::IndexMap;

use crate::complexity_scoring::tier::{ComplexityTier, Tier};
use crate::complexity_turn_overrides::settings::Settings;
use crate::semantic_router::service::{CallerContext, RouteEmbedder};
use crate::tier_definitions::model::{TierDefinition, TierModelEntry, TierPool, TierSet};

pub(crate) const BUILT_IN: [&str; 4] = ["SIMPLE", "MEDIUM", "COMPLEX", "REASONING"];

pub(crate) fn catalog(pools: &[(&str, &[&str])]) -> TierSet {
    let mut resolved = BTreeMap::new();
    for (name, models) in pools {
        resolved.insert((*name).to_owned(), pool(models));
    }
    TierSet {
        definitions: None,
        pools: resolved,
        labels: BTreeMap::new(),
        non_reasoning: false,
        fallback: None,
    }
}

// a custom tier set: the definitions carry the ascending severity order the ladder no longer provides
pub(crate) fn custom_catalog(names: &[&str], pools: &[(&str, &[&str])]) -> TierSet {
    let mut set = catalog(pools);
    set.definitions = Some(
        names
            .iter()
            .map(|name| TierDefinition {
                name: (*name).to_owned(),
                description: None,
            })
            .collect(),
    );
    set
}

fn pool(models: &[&str]) -> TierPool {
    TierPool::Pool(
        models
            .iter()
            .map(|model| TierModelEntry {
                model_name: (*model).to_owned(),
                params: IndexMap::new(),
            })
            .collect(),
    )
}

pub(crate) fn tier(name: &str) -> Tier {
    Tier::BuiltIn(ComplexityTier::built_in(name).expect("the test names a built-in tier"))
}

pub(crate) fn request_kwargs(body: Json) -> IndexMap<String, Json> {
    IndexMap::from([(
        "proxy_server_request".to_owned(),
        json_object(&[("body", body)]),
    )])
}

pub(crate) fn rule(keywords: &[&str], tier: &str) -> KeywordTierRule {
    KeywordTierRule {
        keywords: keywords
            .iter()
            .map(|keyword| (*keyword).to_owned())
            .collect(),
        tier: tier.to_owned(),
    }
}

pub(crate) fn rules(spec: &[(&[&str], &str)]) -> Vec<KeywordTierRule> {
    spec.iter()
        .map(|(keywords, tier)| rule(keywords, tier))
        .collect()
}

pub(crate) fn text_rule_settings(keywords: &[&str], tier: &str) -> Settings {
    Settings {
        keyword_tier_rules: vec![rule(keywords, tier)],
        ..Settings::default()
    }
}

pub(crate) fn semantic_settings(spec: &[(&[&str], &str)], threshold: f64) -> Settings {
    Settings {
        keyword_tier_rules: rules(spec),
        keyword_matching: KeywordMatching::Semantic,
        embedding_model: Some("fake-embed".to_owned()),
        match_threshold: threshold,
        ..Settings::default()
    }
}

pub(crate) fn chat(role: Role, value: &str) -> ChatMessage {
    ChatMessage {
        role,
        content: Some(MessageContent::Text(value.to_owned())),
        name: None,
        tool_calls: None,
        tool_call_id: None,
        passthrough: IndexMap::new(),
    }
}

pub(crate) fn parts(role: Role, content: Vec<ContentPart>) -> ChatMessage {
    ChatMessage {
        role,
        content: Some(MessageContent::Parts(content)),
        name: None,
        tool_calls: None,
        tool_call_id: None,
        passthrough: IndexMap::new(),
    }
}

pub(crate) fn text_part(value: Json) -> ContentPart {
    ContentPart {
        kind: "text".to_owned(),
        fields: IndexMap::from([(
            "text".to_owned(),
            serde_json::to_value(value).expect("a JSON value serializes"),
        )]),
    }
}

pub(crate) fn json_text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(crate) fn json_number(value: i64) -> Json {
    Json::Number(serde_json::Number::from(value))
}

pub(crate) fn json_object(pairs: &[(&str, Json)]) -> Json {
    Json::Object(
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect(),
    )
}

pub(crate) fn json_list(items: Vec<Json>) -> Json {
    Json::List(items)
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum FakeError {
    #[error("the fake embedder has no vector for {text:?}")]
    Missing { text: String },
    #[error("the fake embedder was told to fail")]
    Injected,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DocumentCall {
    pub(crate) model: String,
    pub(crate) texts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct QueryCall {
    pub(crate) model: String,
    pub(crate) text: String,
    pub(crate) caller: CallerContext,
}

pub(crate) struct FakeEmbedder {
    vectors: BTreeMap<String, Vec<f64>>,
    documents: Mutex<Vec<DocumentCall>>,
    queries: Mutex<Vec<QueryCall>>,
    document_failures: AtomicUsize,
    query_failure: bool,
    stall_query: bool,
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
            document_failures: AtomicUsize::new(0),
            query_failure: false,
            stall_query: false,
        }
    }

    pub(crate) fn failing_documents(mut self) -> Self {
        self.document_failures = AtomicUsize::new(1);
        self
    }

    pub(crate) fn failing_query(mut self) -> Self {
        self.query_failure = true;
        self
    }

    pub(crate) fn stalled_query(mut self) -> Self {
        self.stall_query = true;
        self
    }

    pub(crate) fn documents(&self) -> Vec<DocumentCall> {
        self.documents.lock().unwrap().clone()
    }

    pub(crate) fn queries(&self) -> Vec<QueryCall> {
        self.queries.lock().unwrap().clone()
    }
}

impl RouteEmbedder for FakeEmbedder {
    type Error = FakeError;

    fn embed_documents(
        &self,
        model: &str,
        texts: Vec<String>,
    ) -> impl Future<Output = Result<Vec<Vec<f64>>, Self::Error>> + Send {
        async move {
            self.documents.lock().unwrap().push(DocumentCall {
                model: model.to_owned(),
                texts: texts.clone(),
            });
            if take_failure(&self.document_failures) {
                return Err(FakeError::Injected);
            }
            answer(&self.vectors, texts)
        }
    }

    fn embed_query(
        &self,
        model: &str,
        text: String,
        _raw_text: &str,
        caller: &CallerContext,
    ) -> impl Future<Output = Result<Vec<Vec<f64>>, Self::Error>> + Send {
        async move {
            self.queries.lock().unwrap().push(QueryCall {
                model: model.to_owned(),
                text: text.clone(),
                caller: caller.clone(),
            });
            if self.stall_query {
                return pending().await;
            }
            if self.query_failure {
                return Err(FakeError::Injected);
            }
            answer(&self.vectors, vec![text])
        }
    }
}

fn answer(
    vectors: &BTreeMap<String, Vec<f64>>,
    texts: Vec<String>,
) -> Result<Vec<Vec<f64>>, FakeError> {
    let mut found = Vec::with_capacity(texts.len());
    for text in texts {
        let vector = vectors
            .get(&text)
            .cloned()
            .ok_or(FakeError::Missing { text })?;
        found.push(vector);
    }
    Ok(found)
}

fn take_failure(counter: &AtomicUsize) -> bool {
    counter
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
            remaining.checked_sub(1)
        })
        .is_ok()
}
