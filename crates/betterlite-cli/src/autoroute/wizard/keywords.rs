use super::{Prompter, WizardError};
use crate::autoroute::config::semantic::{KeywordTierRule, default_keywords};
use crate::autoroute::config::tier::Tier;

const KEYWORDS_INTRO: &str =
    "\nEnter example keywords/phrases per tier (comma-separated); press enter to keep the default:";

#[must_use]
pub(crate) fn parse_keywords(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|keyword| !keyword.is_empty())
        .map(str::to_owned)
        .collect()
}

pub(crate) fn prompt_keyword_tier_rules(
    prompter: &mut dyn Prompter,
) -> Result<Vec<KeywordTierRule>, WizardError> {
    prompter.announce(KEYWORDS_INTRO)?;
    let mut rules = Vec::with_capacity(Tier::ALL.len());
    for tier in Tier::ALL {
        let defaults = default_keywords(tier);
        let default_text = defaults.join(", ");
        let message = format!("  {tier} keywords");
        let raw = prompter.prompt_keywords(&message, &default_text)?;
        let mut keywords = parse_keywords(&raw);
        if keywords.is_empty() {
            keywords = defaults
                .iter()
                .map(|keyword| (*keyword).to_owned())
                .collect();
        }
        rules.push(KeywordTierRule { keywords, tier });
    }
    Ok(rules)
}
