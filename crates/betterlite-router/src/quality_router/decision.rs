use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use crate::complexity_scoring::classify::ClassificationOutcome;

use super::error::QualityRouterError;

pub(crate) const DECISION_KEY: &str = "quality_router_decision";

// StandardLoggingRoutingDecision for the two routed paths; the default fallback carries no tier facts
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    Keyword {
        router_model_name: String,
        routed_model: String,
        matched_keyword: String,
        quality_tier: i64,
    },
    QualityTier {
        router_model_name: String,
        routed_model: String,
        quality_tier: i64,
        complexity_tier: String,
        score: f64,
        signals: Vec<String>,
    },
    DefaultFallback {
        router_model_name: String,
        routed_model: String,
    },
}

impl Decision {
    #[must_use]
    pub fn keyword(
        router_model_name: &str,
        routed_model: &str,
        matched_keyword: &str,
        quality_tier: i64,
    ) -> Self {
        Self::Keyword {
            router_model_name: router_model_name.to_owned(),
            routed_model: routed_model.to_owned(),
            matched_keyword: matched_keyword.to_owned(),
            quality_tier,
        }
    }

    #[must_use]
    pub fn quality(
        router_model_name: &str,
        routed_model: &str,
        quality_tier: i64,
        outcome: &ClassificationOutcome,
    ) -> Self {
        Self::QualityTier {
            router_model_name: router_model_name.to_owned(),
            routed_model: routed_model.to_owned(),
            quality_tier,
            complexity_tier: outcome.tier.name().to_owned(),
            score: outcome.score,
            signals: outcome.signals.clone(),
        }
    }

    #[must_use]
    pub fn default_fallback(router_model_name: &str, routed_model: &str) -> Self {
        Self::DefaultFallback {
            router_model_name: router_model_name.to_owned(),
            routed_model: routed_model.to_owned(),
        }
    }

    pub fn record(&self) -> Result<Json, QualityRouterError> {
        let record = match self {
            Self::Keyword {
                router_model_name,
                routed_model,
                matched_keyword,
                quality_tier,
            } => keyword_record(
                router_model_name,
                routed_model,
                matched_keyword,
                *quality_tier,
            ),
            Self::QualityTier {
                router_model_name,
                routed_model,
                quality_tier,
                score,
                signals,
                ..
            } => quality_record(
                router_model_name,
                routed_model,
                *quality_tier,
                *score,
                signals,
            )?,
            Self::DefaultFallback {
                router_model_name,
                routed_model,
            } => record_base(router_model_name, routed_model, "default_fallback"),
        };
        Ok(Json::Object(record))
    }

    // the metadata the header helper lifts into x-litellm-quality-router-*; the default fallback stashes none
    pub fn stash_document(&self) -> Option<Json> {
        let document = match self {
            Self::Keyword {
                router_model_name,
                routed_model,
                matched_keyword,
                quality_tier,
            } => keyword_document(
                router_model_name,
                routed_model,
                matched_keyword,
                *quality_tier,
            ),
            Self::QualityTier {
                router_model_name,
                routed_model,
                quality_tier,
                complexity_tier,
                ..
            } => quality_document(
                router_model_name,
                routed_model,
                *quality_tier,
                complexity_tier,
            ),
            Self::DefaultFallback { .. } => return None,
        };
        Some(Json::Object(document))
    }
}

fn keyword_record(
    router_model_name: &str,
    routed_model: &str,
    matched_keyword: &str,
    quality_tier: i64,
) -> IndexMap<String, Json> {
    let mut record = record_base(router_model_name, routed_model, "keyword");
    record.insert(
        "matched_keyword".to_owned(),
        Json::Text(matched_keyword.to_owned()),
    );
    record.insert("tier".to_owned(), Json::Text(quality_tier.to_string()));
    record
}

fn quality_record(
    router_model_name: &str,
    routed_model: &str,
    quality_tier: i64,
    score: f64,
    signals: &[String],
) -> Result<IndexMap<String, Json>, QualityRouterError> {
    let mut record = record_base(router_model_name, routed_model, "quality_tier");
    record.insert("tier".to_owned(), Json::Text(quality_tier.to_string()));
    let number = Number::from_f64(score).ok_or(QualityRouterError::NonFiniteScore)?;
    record.insert("score".to_owned(), Json::Number(number));
    record.insert(
        "signals".to_owned(),
        Json::List(signals.iter().cloned().map(Json::Text).collect()),
    );
    Ok(record)
}

fn record_base(router_model_name: &str, routed_model: &str, cause: &str) -> IndexMap<String, Json> {
    let mut record = IndexMap::new();
    record.insert(
        "router_model_name".to_owned(),
        Json::Text(router_model_name.to_owned()),
    );
    record.insert("router_type".to_owned(), Json::Text("quality".to_owned()));
    record.insert(
        "routed_model".to_owned(),
        Json::Text(routed_model.to_owned()),
    );
    record.insert("cause".to_owned(), Json::Text(cause.to_owned()));
    record
}

fn keyword_document(
    router_model_name: &str,
    routed_model: &str,
    matched_keyword: &str,
    quality_tier: i64,
) -> IndexMap<String, Json> {
    let mut document = document_base(router_model_name, routed_model);
    document.insert("routed_via".to_owned(), Json::Text("keyword".to_owned()));
    document.insert(
        "matched_keyword".to_owned(),
        Json::Text(matched_keyword.to_owned()),
    );
    document.insert(
        "quality_tier".to_owned(),
        Json::Number(Number::from(quality_tier)),
    );
    document.insert("complexity_tier".to_owned(), Json::Null);
    document
}

fn quality_document(
    router_model_name: &str,
    routed_model: &str,
    quality_tier: i64,
    complexity_tier: &str,
) -> IndexMap<String, Json> {
    let mut document = document_base(router_model_name, routed_model);
    document.insert(
        "routed_via".to_owned(),
        Json::Text("quality_tier".to_owned()),
    );
    document.insert("matched_keyword".to_owned(), Json::Null);
    document.insert(
        "quality_tier".to_owned(),
        Json::Number(Number::from(quality_tier)),
    );
    document.insert(
        "complexity_tier".to_owned(),
        Json::Text(complexity_tier.to_owned()),
    );
    document
}

fn document_base(router_model_name: &str, routed_model: &str) -> IndexMap<String, Json> {
    let mut document = IndexMap::new();
    document.insert(
        "router_model_name".to_owned(),
        Json::Text(router_model_name.to_owned()),
    );
    document.insert(
        "routed_model".to_owned(),
        Json::Text(routed_model.to_owned()),
    );
    document
}

// an absent metadata is created and an existing object takes the new decision; any other value is left alone
pub(crate) fn stash(metadata: &mut Option<Json>, decision: Json) {
    match metadata {
        None => {
            let mut fields = IndexMap::new();
            fields.insert(DECISION_KEY.to_owned(), decision);
            *metadata = Some(Json::Object(fields));
        }
        Some(Json::Object(fields)) => {
            fields.insert(DECISION_KEY.to_owned(), decision);
        }
        Some(_) => {}
    }
}
