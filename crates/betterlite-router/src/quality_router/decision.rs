use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Serialize;

use super::error::QualityRouterError;

pub(crate) const DECISION_KEY: &str = "quality_router_decision";

const ROUTER_TYPE: &str = "quality";

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

// the record a StandardLoggingRoutingDecision dumps; absent facts stay out of it, as the TypedDict does
#[derive(Serialize)]
struct RoutingRecord<'decision> {
    router_model_name: &'decision str,
    router_type: &'static str,
    routed_model: &'decision str,
    cause: RoutedBy,
    #[serde(skip_serializing_if = "Option::is_none")]
    matched_keyword: Option<&'decision str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    signals: Option<&'decision [String]>,
}

// the decision stash the header helper lifts into x-litellm-quality-router-*; nulls are written as null
#[derive(Serialize)]
struct DecisionDocument<'decision> {
    router_model_name: &'decision str,
    routed_model: &'decision str,
    routed_via: RoutedBy,
    matched_keyword: Option<&'decision str>,
    quality_tier: i64,
    complexity_tier: Option<&'decision str>,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum RoutedBy {
    Keyword,
    QualityTier,
    DefaultFallback,
}

// the fields both wire types read from a decision; a fact the cause does not carry stays None
struct Facts<'decision> {
    router_model_name: &'decision str,
    routed_model: &'decision str,
    routed_by: RoutedBy,
    matched_keyword: Option<&'decision str>,
    quality_tier: Option<i64>,
    complexity_tier: Option<&'decision str>,
    score: Option<f64>,
    signals: Option<&'decision [String]>,
}

impl Decision {
    fn facts(&self) -> Facts<'_> {
        match self {
            Self::Keyword {
                router_model_name,
                routed_model,
                matched_keyword,
                quality_tier,
            } => Facts {
                router_model_name,
                routed_model,
                routed_by: RoutedBy::Keyword,
                matched_keyword: Some(matched_keyword),
                quality_tier: Some(*quality_tier),
                complexity_tier: None,
                score: None,
                signals: None,
            },
            Self::QualityTier {
                router_model_name,
                routed_model,
                quality_tier,
                complexity_tier,
                score,
                signals,
            } => Facts {
                router_model_name,
                routed_model,
                routed_by: RoutedBy::QualityTier,
                matched_keyword: None,
                quality_tier: Some(*quality_tier),
                complexity_tier: Some(complexity_tier),
                score: Some(*score),
                signals: Some(signals),
            },
            Self::DefaultFallback {
                router_model_name,
                routed_model,
            } => Facts {
                router_model_name,
                routed_model,
                routed_by: RoutedBy::DefaultFallback,
                matched_keyword: None,
                quality_tier: None,
                complexity_tier: None,
                score: None,
                signals: None,
            },
        }
    }

    pub fn record(&self) -> Result<Json, QualityRouterError> {
        let facts = self.facts();
        if let Some(score) = facts.score
            && !score.is_finite()
        {
            return Err(QualityRouterError::NonFiniteScore);
        }
        wire(&RoutingRecord {
            router_model_name: facts.router_model_name,
            router_type: ROUTER_TYPE,
            routed_model: facts.routed_model,
            cause: facts.routed_by,
            matched_keyword: facts.matched_keyword,
            tier: facts.quality_tier.map(|tier| tier.to_string()),
            score: facts.score,
            signals: facts.signals,
        })
    }

    // only the two routed paths carry a quality tier, and the default fallback stashes nothing
    pub fn stash_document(&self) -> Result<Option<Json>, QualityRouterError> {
        let facts = self.facts();
        let Some(quality_tier) = facts.quality_tier else {
            return Ok(None);
        };
        wire(&DecisionDocument {
            router_model_name: facts.router_model_name,
            routed_model: facts.routed_model,
            routed_via: facts.routed_by,
            matched_keyword: facts.matched_keyword,
            quality_tier,
            complexity_tier: facts.complexity_tier,
        })
        .map(Some)
    }
}

// the wire types stop at the passthrough document: serde's field order is kept by reading the text back
fn wire<T>(value: &T) -> Result<Json, QualityRouterError>
where
    T: Serialize + ?Sized,
{
    let text = serde_json::to_string(value)?;
    let document: Json = serde_json::from_str(&text)?;
    Ok(document)
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
