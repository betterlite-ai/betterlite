use super::configuration::Settings;
use super::{
    BaseCompetitorIntentChecker, Error, Meaning, OTHER_MEANING_THRESHOLD, normalize,
    word_boundary_match,
};
use betterlite_config::sections::content_filter_records::overrides::CompetitorIntentEvidenceEntry;
use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};

#[must_use]
pub(super) fn find_matches(
    checker: &BaseCompetitorIntentChecker,
    text: &str,
) -> Vec<(String, String, bool)> {
    let normalized = normalize(text);
    checker
        .competitor_tokens
        .iter()
        .filter(|token| word_boundary_match(&normalized, token))
        .filter_map(|token| {
            checker.competitor_canonical.get(token).map(|canonical| {
                (
                    token.clone(),
                    canonical.clone(),
                    checker.ambiguous.contains(token),
                )
            })
        })
        .collect()
}

pub(super) fn resolve_matches<F>(
    normalized: &str,
    matches: Vec<(String, String, bool)>,
    classify: F,
) -> (Vec<String>, Vec<CompetitorIntentEvidenceEntry>)
where
    F: Fn(&str, &str) -> (Meaning, f64),
{
    let mut competitors = Vec::new();
    let mut evidence = Vec::new();
    for (token, canonical, _) in matches {
        let (meaning, confidence) = classify(normalized, &token);
        if meaning == Meaning::OtherMeaning {
            evidence.push(CompetitorIntentEvidenceEntry {
                r#type: Some("signal".to_owned()),
                key: Some("other_meaning".to_owned()),
                r#match: Some(token),
                value: None,
            });
            continue;
        }
        evidence.push(CompetitorIntentEvidenceEntry {
            r#type: Some("entity".to_owned()),
            key: Some("competitor".to_owned()),
            r#match: Some(token),
            value: Some(canonical.clone()),
        });
        if confidence < OTHER_MEANING_THRESHOLD {
            evidence.push(CompetitorIntentEvidenceEntry {
                r#type: Some("signal".to_owned()),
                key: Some("other_meaning_default".to_owned()),
                r#match: Some(format!("confidence {confidence:.2}")),
                value: None,
            });
        } else {
            competitors.push(canonical);
        }
    }
    (competitors, evidence)
}
pub(super) fn normalized_set(values: &[String]) -> BTreeSet<String> {
    let values_iter = values.iter();
    values_iter
        .filter(|value| !value.is_empty())
        .map(|value| value.trim().to_lowercase())
        .collect()
}

pub(super) fn build_competitor_maps(
    config: &Settings,
) -> Result<(BTreeMap<String, String>, BTreeSet<String>), Error> {
    let mut canonical_map = BTreeMap::new();
    let mut tokens = BTreeSet::new();
    for competitor in &config.competitors {
        if competitor.is_empty() {
            continue;
        }
        let canonical = competitor.trim().to_lowercase();
        tokens.insert(canonical.clone());
        insert_canonical(&mut canonical_map, canonical.clone(), &canonical)?;
        for alias in config
            .competitor_aliases
            .get(&canonical)
            .into_iter()
            .flatten()
        {
            let token = alias.trim().to_lowercase();
            if !token.is_empty() {
                tokens.insert(token.clone());
                insert_canonical(&mut canonical_map, token, &canonical)?;
            }
        }
    }
    Ok((canonical_map, tokens))
}

fn insert_canonical(
    canonical_map: &mut BTreeMap<String, String>,
    token: String,
    canonical: &str,
) -> Result<(), Error> {
    match canonical_map.entry(token) {
        Entry::Occupied(entry) => {
            if entry.get() != canonical {
                return Err(Error::TokenCollision {
                    competitor: canonical.to_owned(),
                    token: entry.key().clone(),
                    existing: entry.get().clone(),
                });
            }
        }
        Entry::Vacant(entry) => {
            entry.insert(canonical.to_owned());
        }
    }
    Ok(())
}
