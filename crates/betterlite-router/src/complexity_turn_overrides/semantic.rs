use std::future::Future;
use std::sync::Arc;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use tokio::sync::OnceCell;
use tokio::time::timeout;

use crate::complexity_scoring::tier::Tier;
use crate::semantic_math::{MathError, cosine_similarity, validate_documents};
use crate::semantic_router::index::{RouteIndex, RouteIndexError, parse_inline};
use crate::semantic_router::select::{BuiltEntry, TOP_K};
use crate::semantic_router::service::{CallerContext, EMBEDDING_TIMEOUT, RouteEmbedder};
use crate::tier_definitions::model::TierSet;

use super::settings::Settings;

// the encoder probes the embedding width with this text before it embeds any utterance
const DIMENSION_PROBE_TEXT: &str = "test";

#[derive(Debug, thiserror::Error)]
pub enum SemanticError {
    #[error("embedding_model is required for semantic keyword matching")]
    MissingModel,
    #[error("match_threshold must be between 0.0 and 1.0")]
    Threshold,
    #[error("the semantic keyword routes could not be serialized: {0}")]
    Config(#[from] serde_json::Error),
    #[error("the semantic keyword route index is unusable: {0}")]
    Index(#[from] RouteIndexError),
    #[error("the {stage} embedding call failed: {source}")]
    Embedding { stage: &'static str, source: String },
    #[error("the {stage} embedding call did not finish within {seconds} seconds")]
    Timeout { stage: &'static str, seconds: u64 },
    #[error("the {stage} embedding call returned {found} vectors for {expected} inputs")]
    VectorCount {
        stage: &'static str,
        expected: usize,
        found: usize,
    },
    #[error("the embedding vectors are unusable: {0}")]
    Vectors(#[from] MathError),
    #[error("the query embedding call returned no vector")]
    EmptyQuery,
}

// one route per tier whose rules name it, utterances in rule order, built once and only read afterwards
pub struct SemanticKeywordIndex<E>
where
    E: RouteEmbedder + 'static,
{
    model: String,
    catalog: TierSet,
    index: RouteIndex,
    embedder: Arc<E>,
    built: OnceCell<Arc<[BuiltEntry]>>,
}

impl<E> SemanticKeywordIndex<E>
where
    E: RouteEmbedder + 'static,
{
    pub fn new(
        settings: &Settings,
        catalog: &TierSet,
        embedder: Arc<E>,
    ) -> Result<Self, SemanticError> {
        let model = settings
            .embedding_model
            .clone()
            .ok_or(SemanticError::MissingModel)?;
        let index = parse_inline(&route_config(settings)?)?;
        Ok(Self {
            model,
            catalog: catalog.clone(),
            index,
            embedder,
            built: OnceCell::new(),
        })
    }

    // _semantic_tier_override: a similarity hit against the rule utterances, None on any failure
    pub async fn tier_override(&self, user_message: &str, caller: &CallerContext) -> Option<Tier> {
        match self.try_tier_override(user_message, caller).await {
            Ok(tier) => tier,
            Err(error) => {
                tracing::warn!(
                    %error,
                    "semantic keyword matching failed, falling back to complexity scoring"
                );
                None
            }
        }
    }

    async fn try_tier_override(
        &self,
        user_message: &str,
        caller: &CallerContext,
    ) -> Result<Option<Tier>, SemanticError> {
        let entries = self.built.get_or_try_init(|| self.build_entries()).await?;
        let vectors = self.query_batch(user_message, caller).await?;
        let Some(query) = vectors.into_iter().next() else {
            return Err(SemanticError::EmptyQuery);
        };
        let matched = select_route(entries, &query)?;
        Ok(matched.and_then(|name| self.catalog.resolve_classified_tier(&name)))
    }

    // the build embeds the static utterances once, through the same document path the route sync uses
    async fn build_entries(&self) -> Result<Arc<[BuiltEntry]>, SemanticError> {
        let _probe = self
            .document_batch(vec![DIMENSION_PROBE_TEXT.to_owned()])
            .await?;
        let texts: Vec<String> = self.index.rows.iter().map(|row| row.text.clone()).collect();
        if texts.is_empty() {
            return Ok(Arc::<[BuiltEntry]>::from(Vec::new()));
        }
        let vectors = self.document_batch(texts).await?;
        validate_documents(&vectors)?;
        let mut entries = Vec::with_capacity(self.index.rows.len());
        for (row, vector) in self.index.rows.iter().zip(vectors) {
            entries.push(BuiltEntry {
                route: Arc::clone(&row.route),
                threshold: row.threshold,
                vector,
            });
        }
        Ok(Arc::from(entries))
    }

    async fn document_batch(&self, texts: Vec<String>) -> Result<Vec<Vec<f64>>, SemanticError> {
        let expected = texts.len();
        let vectors = self
            .wait(
                "documents",
                self.embedder.embed_documents(&self.model, texts),
            )
            .await?;
        if vectors.len() != expected {
            return Err(SemanticError::VectorCount {
                stage: "documents",
                expected,
                found: vectors.len(),
            });
        }
        Ok(vectors)
    }

    async fn query_batch(
        &self,
        user_message: &str,
        caller: &CallerContext,
    ) -> Result<Vec<Vec<f64>>, SemanticError> {
        self.wait(
            "query",
            self.embedder
                .embed_query(&self.model, user_message.to_owned(), user_message, caller),
        )
        .await
    }

    async fn wait<F>(&self, stage: &'static str, answer: F) -> Result<Vec<Vec<f64>>, SemanticError>
    where
        F: Future<Output = Result<Vec<Vec<f64>>, E::Error>>,
    {
        match timeout(EMBEDDING_TIMEOUT, answer).await {
            Ok(Ok(vectors)) => Ok(vectors),
            Ok(Err(source)) => Err(SemanticError::Embedding {
                stage,
                source: source.to_string(),
            }),
            Err(_elapsed) => Err(SemanticError::Timeout {
                stage,
                seconds: EMBEDDING_TIMEOUT.as_secs(),
            }),
        }
    }
}

// _get_or_create_semantic_routelayer aggregates with max: a route scores by its best utterance
fn select_route(entries: &[BuiltEntry], query: &[f64]) -> Result<Option<String>, MathError> {
    let mut scored: Vec<(&str, f64, f64)> = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let score = cosine_similarity(query, &entry.vector, index)?;
        scored.push((entry.route.as_ref(), entry.threshold, score));
    }
    // the source's index query returns the top_k scores ascending, ties keeping row order
    scored.sort_by(|left, right| right.2.total_cmp(&left.2));
    scored.truncate(TOP_K.min(scored.len()));
    scored.sort_by(|left, right| left.2.total_cmp(&right.2));
    let mut routes: Vec<(&str, f64, f64)> = Vec::new();
    for (name, threshold, score) in scored {
        if let Some(position) = routes.iter().position(|(found, _, _)| *found == name) {
            if let Some((_, _, best)) = routes.get_mut(position) {
                *best = best.max(score);
            }
        } else {
            routes.push((name, threshold, score));
        }
    }
    routes.sort_by(|left, right| right.2.total_cmp(&left.2));
    for (name, threshold, best) in routes {
        // a zero threshold is falsy in the source and always passes
        if threshold == 0.0_f64 || best >= threshold {
            return Ok(Some(name.to_owned()));
        }
    }
    Ok(None)
}

// _get_or_create_semantic_routelayer's route list: the first tier each rule names, in rule order
fn route_config(settings: &Settings) -> Result<String, SemanticError> {
    let threshold = settings.match_threshold;
    if !threshold.is_finite() || threshold < 0.0_f64 || threshold > 1.0_f64 {
        return Err(SemanticError::Threshold);
    }
    let Some(number) = serde_json::Number::from_f64(threshold) else {
        return Err(SemanticError::Threshold);
    };
    let routes: Vec<Json> = tier_order(settings)
        .into_iter()
        .map(|tier| route(settings, tier, &number))
        .collect();
    let document = Json::Object(IndexMap::from([("routes".to_owned(), Json::List(routes))]));
    Ok(serde_json::to_string(&document)?)
}

fn tier_order(settings: &Settings) -> Vec<&str> {
    let mut tiers: Vec<&str> = Vec::new();
    for rule in &settings.keyword_tier_rules {
        if !tiers.contains(&rule.tier.as_str()) {
            tiers.push(rule.tier.as_str());
        }
    }
    tiers
}

// one route per tier: every keyword of every rule naming it, in rule order
fn route(settings: &Settings, tier: &str, number: &serde_json::Number) -> Json {
    let mut utterances = Vec::new();
    for rule in &settings.keyword_tier_rules {
        if rule.tier.as_str() != tier {
            continue;
        }
        for keyword in &rule.keywords {
            utterances.push(Json::Text(keyword.clone()));
        }
    }
    Json::Object(IndexMap::from([
        ("name".to_owned(), Json::Text(tier.to_owned())),
        ("utterances".to_owned(), Json::List(utterances)),
        ("score_threshold".to_owned(), Json::Number(number.clone())),
    ]))
}
