use betterlite_config::sections::complexity_turn_overrides::KeywordTierRule;

use crate::complexity_scoring::keywords::KeywordList;
use crate::complexity_scoring::tier::Tier;
use crate::tier_definitions::model::TierSet;

use super::error::TurnOverrideError;
use super::settings::{Settings, active_tier, tier_severity};

// LiteLLM's default escalation phrase; an unset list means this one, an empty list turns it off
pub const DEFAULT_ESCALATION_KEYWORDS: [&str; 1] = ["LITELLM ESCALATE"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeywordOverride {
    pub tier: Tier,
    // None for a semantic hit: a similarity match is not a literal keyword
    pub matched_keyword: Option<String>,
}

struct CompiledRule {
    keywords: KeywordList,
    tier: Tier,
    severity: usize,
}

// the keyword_tier_rules compiled once, with each rule's tier resolved against the active tier order
pub struct LexicalRules {
    rules: Vec<CompiledRule>,
}

impl LexicalRules {
    pub fn compile(
        catalog: &TierSet,
        rules: &[KeywordTierRule],
    ) -> Result<Self, TurnOverrideError> {
        let mut compiled = Vec::with_capacity(rules.len());
        for rule in rules {
            let tier = active_tier(catalog, rule.tier.as_str())?;
            let severity = tier_severity(catalog, &tier)?;
            compiled.push(CompiledRule {
                keywords: KeywordList::compile(&rule.keywords)?,
                tier,
                severity,
            });
        }
        Ok(Self { rules: compiled })
    }

    // _lexical_tier_override: the most-severe matched rule wins, whichever order the rules were authored in
    #[must_use]
    pub fn matches(&self, user_message: &str) -> Option<KeywordOverride> {
        let text = user_message.to_lowercase();
        let mut best: Option<KeywordOverride> = None;
        let mut best_severity = 0;
        for rule in &self.rules {
            let Some(matched) = rule.keywords.matches(&text).into_iter().next() else {
                continue;
            };
            // max keeps the first maximal item, so a later rule of equal severity never displaces it
            if best.is_none() || rule.severity > best_severity {
                best_severity = rule.severity;
                best = Some(KeywordOverride {
                    tier: rule.tier.clone(),
                    matched_keyword: Some(matched.to_owned()),
                });
            }
        }
        best
    }
}

// _matched_escalation_keyword: a case-sensitive substring, off for an empty list or a custom tier set
#[must_use]
pub fn matched_escalation_keyword(
    settings: &Settings,
    catalog: &TierSet,
    user_message: &str,
) -> Option<String> {
    if catalog.definitions.is_some() {
        return None;
    }
    match settings.escalation_keywords.as_deref() {
        Some(keywords) if keywords.is_empty() => None,
        Some(keywords) => keywords
            .iter()
            .find(|keyword| user_message.contains(keyword.as_str()))
            .cloned(),
        None => DEFAULT_ESCALATION_KEYWORDS
            .iter()
            .copied()
            .find(|keyword| user_message.contains(keyword))
            .map(str::to_owned),
    }
}
