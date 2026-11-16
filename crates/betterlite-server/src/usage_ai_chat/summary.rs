use betterlite_analytics::daily_metrics::{DailySpendPage, SpendMetrics};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use indexmap::map::Entry;
use rust_decimal::Decimal;
use serde::Deserialize;

use super::constants::{TOP_N_MODELS, TOP_N_PROVIDERS};
use super::error::UsageChatError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityLabel {
    Team,
    Tag,
}

impl EntityLabel {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Team => "Team",
            Self::Tag => "Tag",
        }
    }

    #[must_use]
    pub const fn plural(self) -> &'static str {
        match self {
            Self::Team => "teams",
            Self::Tag => "tags",
        }
    }
}

#[derive(Debug, Default)]
struct Totals {
    spend: Decimal,
    api_requests: i64,
    total_tokens: i64,
}

impl Totals {
    fn add(&mut self, metrics: &SpendMetrics) -> Result<(), UsageChatError> {
        self.spend = self
            .spend
            .checked_add(metrics.spend)
            .ok_or(UsageChatError::Overflow)?;
        self.api_requests = self
            .api_requests
            .checked_add(metrics.api_requests)
            .ok_or(UsageChatError::Overflow)?;
        self.total_tokens = self
            .total_tokens
            .checked_add(metrics.total_tokens)
            .ok_or(UsageChatError::Overflow)?;
        Ok(())
    }
}

#[derive(Debug)]
struct EntityTotals {
    alias: String,
    totals: Totals,
}

pub fn summarise_usage_data(page: &DailySpendPage) -> Result<String, UsageChatError> {
    let metadata = &page.metadata;
    let header = format!(
        "Total Spend: ${:.4}\nTotal Requests: {}\nSuccessful: {} | Failed: {}\nTotal Tokens: {}",
        metadata.total_spend.round_dp(4),
        metadata.total_api_requests,
        metadata.total_successful_requests,
        metadata.total_failed_requests,
        metadata.total_tokens,
    );
    let (models, providers) = accumulate_breakdowns(page)?;
    let model_lines = ranked_lines(&models, TOP_N_MODELS, |name, totals| {
        format!(
            "  - {name}: ${:.4} ({} reqs, {} tokens)",
            totals.spend.round_dp(4),
            totals.api_requests,
            totals.total_tokens,
        )
    });
    let provider_lines = ranked_lines(&providers, TOP_N_PROVIDERS, |name, totals| {
        format!(
            "  - {name}: ${:.4} ({} reqs)",
            totals.spend.round_dp(4),
            totals.api_requests,
        )
    });
    let mut sections: Vec<String> = vec![header, String::new(), "Top Models by Spend:".to_owned()];
    sections.extend(listing(model_lines));
    sections.push(String::new());
    sections.push("Top Providers by Spend:".to_owned());
    sections.extend(listing(provider_lines));
    Ok(sections.join("\n"))
}

pub fn summarise_entity_data(
    page: &DailySpendPage,
    label: EntityLabel,
) -> Result<String, UsageChatError> {
    if page.results.is_empty() {
        return Ok(format!(
            "No {} usage data found for the given date range.",
            label.as_str()
        ));
    }
    let totals = entity_totals(page)?;
    let mut lines = vec![
        format!(
            "{} Usage ({} {}):",
            label.as_str(),
            totals.len(),
            label.plural()
        ),
        String::new(),
    ];
    let mut ranked: Vec<(&String, &EntityTotals)> = totals.iter().collect();
    ranked.sort_by(|left, right| right.1.spend.cmp(&left.1.spend));
    for (id, totals) in ranked {
        lines.push(format!(
            "- {} (ID: {id}): ${:.4} | {} reqs | {} tokens",
            totals.alias,
            totals.totals.spend.round_dp(4),
            totals.totals.api_requests,
            totals.totals.total_tokens,
        ));
    }
    Ok(lines.join("\n"))
}

fn accumulate_breakdowns(
    page: &DailySpendPage,
) -> Result<(IndexMap<String, Totals>, IndexMap<String, Totals>), UsageChatError> {
    let mut models: IndexMap<String, Totals> = IndexMap::new();
    let mut providers: IndexMap<String, Totals> = IndexMap::new();
    for day in &page.results {
        for (name, entry) in &day.breakdown.models {
            models
                .entry(name.clone())
                .or_default()
                .add(&entry.metrics)?;
        }
        for (name, entry) in &day.breakdown.providers {
            providers
                .entry(name.clone())
                .or_default()
                .add(&entry.metrics)?;
        }
    }
    Ok((models, providers))
}

fn entity_totals(page: &DailySpendPage) -> Result<IndexMap<String, EntityTotals>, UsageChatError> {
    let mut per_entity: IndexMap<String, EntityTotals> = IndexMap::new();
    for day in &page.results {
        for (id, entry) in &day.breakdown.entities {
            let totals = match per_entity.entry(id.clone()) {
                Entry::Occupied(slot) => slot.into_mut(),
                Entry::Vacant(slot) => slot.insert(EntityTotals {
                    alias: entity_alias(&entry.metadata, id)?,
                    totals: Totals::default(),
                }),
            };
            totals.totals.add(&entry.metrics)?;
        }
    }
    Ok(per_entity)
}

fn entity_alias(metadata: &IndexMap<String, Json>, entity: &str) -> Result<String, UsageChatError> {
    match metadata.get("alias") {
        None | Some(Json::Null) => Ok(entity.to_owned()),
        Some(Json::Text(alias)) => Ok(alias.clone()),
        Some(_) => Err(UsageChatError::EntityAlias {
            entity: entity.to_owned(),
        }),
    }
}

fn ranked_lines<Format>(
    totals: &IndexMap<String, Totals>,
    limit: usize,
    format: Format,
) -> Vec<String>
where
    Format: Fn(&str, &Totals) -> String,
{
    let mut ranked: Vec<(&String, &Totals)> = totals.iter().collect();
    ranked.sort_by(|left, right| right.1.spend.cmp(&left.1.spend));
    ranked.truncate(limit);
    ranked
        .into_iter()
        .map(|(name, totals)| format(name, totals))
        .collect()
}

fn listing(lines: Vec<String>) -> Vec<String> {
    if lines.is_empty() {
        return vec!["  (no data)".to_owned()];
    }
    lines
}
