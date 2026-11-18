use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::ops::Bound::{Excluded, Unbounded};

use betterlite_config::sections::quality_router as section;
use indexmap::{IndexMap, IndexSet};
use rust_decimal::Decimal;

use super::config::Config;
use super::error::QualityRouterError;

// an unset order or cost sorts after every declared value, which is what +inf did in Python
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Priority<T> {
    Set(T),
    Unset,
}

struct ModelEntry {
    keywords: Vec<String>,
    quality_tier: i64,
    order: Priority<i64>,
    cost: Priority<Decimal>,
}

pub struct Index {
    // the best model of each quality tier, by (order, cost, name)
    tiers: BTreeMap<i64, String>,
    // model_list order of first appearance, holding the last declared preferences
    models: IndexMap<String, ModelEntry>,
}

impl Index {
    pub(crate) fn build(
        config: &Config,
        deployments: &[section::Deployment],
        warnings: &mut Vec<String>,
    ) -> Result<Self, QualityRouterError> {
        let available: IndexSet<&str> =
            config.available_models.iter().map(String::as_str).collect();
        let (models, assignments) = declared(deployments, &available, warnings)?;
        let missing = missing_models(&available, &models);
        if !missing.is_empty() {
            return Err(QualityRouterError::MissingModels { models: missing });
        }
        let tiers = tier_winners(&models, assignments);
        Ok(Self { tiers, models })
    }

    pub(crate) fn keyword_match(&self, user_message: &str) -> Option<(&str, &str, i64)> {
        let text = user_message.to_lowercase();
        self.models
            .iter()
            .filter_map(|(name, entry)| {
                entry
                    .keywords
                    .iter()
                    .find(|keyword| text.contains(keyword.as_str()))
                    .map(|keyword| (name.as_str(), keyword.as_str(), entry))
            })
            .min_by_key(|(name, _, entry)| {
                (
                    Reverse(entry.quality_tier),
                    &entry.order,
                    &entry.cost,
                    *name,
                )
            })
            .map(|(name, keyword, entry)| (name, keyword, entry.quality_tier))
    }

    pub(crate) fn model_for_tier<'a>(
        &'a self,
        tier: i64,
        default_model: Option<&'a str>,
    ) -> Result<&'a str, QualityRouterError> {
        if let Some(model) = self.tiers.get(&tier) {
            return Ok(model);
        }
        if let Some((_, model)) = self.tiers.range((Excluded(tier), Unbounded)).next() {
            return Ok(model);
        }
        if let Some((_, model)) = self.tiers.range((Unbounded, Excluded(tier))).next_back() {
            return Ok(model);
        }
        default_model.ok_or(QualityRouterError::NoModelForTier { tier })
    }
}

// one entry per available name, with the last declared preferences winning, and every tier assignment in model_list order
fn declared(
    deployments: &[section::Deployment],
    available: &IndexSet<&str>,
    warnings: &mut Vec<String>,
) -> Result<(IndexMap<String, ModelEntry>, Vec<(i64, String)>), QualityRouterError> {
    let mut models = IndexMap::new();
    let mut assignments = Vec::new();
    for deployment in deployments {
        if !available.contains(deployment.model_name.as_str()) {
            continue;
        }
        let Some(preferences) = deployment.preferences.as_ref() else {
            return Err(QualityRouterError::MissingPreferences {
                model: deployment.model_name.clone(),
            });
        };
        assignments.push((preferences.quality_tier, deployment.model_name.clone()));
        let entry = model_entry(deployment, preferences, warnings);
        models.insert(deployment.model_name.clone(), entry);
    }
    Ok((models, assignments))
}

fn model_entry(
    deployment: &section::Deployment,
    preferences: &section::Preferences,
    warnings: &mut Vec<String>,
) -> ModelEntry {
    let model_name = &deployment.model_name;
    for field in preferences.extras.keys() {
        warnings.push(format!(
            "quality_router: ignoring unknown RoutingPreferences field '{field}' for model '{model_name}'"
        ));
    }
    let keywords = preferences
        .keywords
        .iter()
        .map(|keyword| keyword.to_lowercase())
        .filter(|keyword| !keyword.is_empty())
        .collect();
    let order = preferences.order.map_or(Priority::Unset, Priority::Set);
    let cost = deployment
        .input_cost_per_token
        .map_or(Priority::Unset, Priority::Set);
    ModelEntry {
        keywords,
        quality_tier: preferences.quality_tier,
        order,
        cost,
    }
}

fn missing_models(
    available: &IndexSet<&str>,
    models: &IndexMap<String, ModelEntry>,
) -> Vec<String> {
    available
        .iter()
        .filter(|model| !models.contains_key(**model))
        .map(|model| (*model).to_owned())
        .collect()
}

// a name declared twice sorts by its last preferences, the way the side indices read it
fn tier_winners(
    models: &IndexMap<String, ModelEntry>,
    assignments: Vec<(i64, String)>,
) -> BTreeMap<i64, String> {
    let mut candidates = Vec::with_capacity(assignments.len());
    for (tier, name) in assignments {
        let Some(entry) = models.get(&name) else {
            continue;
        };
        candidates.push((tier, entry.order, entry.cost, name));
    }
    candidates.sort_unstable();
    let mut tiers = BTreeMap::new();
    for (tier, _, _, model) in candidates {
        tiers.entry(tier).or_insert(model);
    }
    tiers
}
