use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;
use serde_json::Number;

// the ComplexityRouterConfig.model_dump(mode="json", include=..., exclude=...) fields tuning_fingerprint hashes
pub(super) fn payload(fields: &JsonObject) -> Option<IndexMap<String, Json>> {
    let mut payload = IndexMap::new();
    for (name, numbers) in [
        ("tier_boundaries", Numbers::Float),
        ("token_thresholds", Numbers::Int),
        ("dimension_weights", Numbers::Float),
    ] {
        if let Some(value) = fields.get(name) {
            payload.insert(name.to_owned(), number_map(value, numbers)?);
        }
    }
    if let Some(value) = fields.get("reasoning_override_min_score") {
        payload.insert(
            "reasoning_override_min_score".to_owned(),
            float_option(value)?,
        );
    }
    let entry_lists: [(&str, fn(&Json) -> Option<Json>); 2] = [
        ("custom_dimensions", dimension_payload),
        ("keyword_tier_rules", keyword_rule),
    ];
    for (name, convert) in entry_lists {
        if let Some(value) = fields.get(name) {
            payload.insert(name.to_owned(), converted_list(value, convert)?);
        }
    }
    insert_keyword_lists(&mut payload, fields)?;
    payload.sort_keys();
    Some(payload)
}

fn insert_keyword_lists(payload: &mut IndexMap<String, Json>, fields: &JsonObject) -> Option<()> {
    for name in [
        "code_keywords",
        "reasoning_keywords",
        "technical_keywords",
        "custom_technical_keywords",
        "simple_keywords",
    ] {
        match text_list(fields.get(name))? {
            TextList::Absent => {}
            TextList::Null => {
                payload.insert(name.to_owned(), Json::Null);
            }
            TextList::Values(texts) => {
                payload.insert(
                    name.to_owned(),
                    Json::List(texts.into_iter().map(Json::Text).collect()),
                );
            }
        }
    }
    // a supplied null takes the model default, the built-in marker the payload leaves out
    if let TextList::Values(texts) = text_list(fields.get("escalation_keywords"))? {
        let cleaned: Vec<String> = texts
            .iter()
            .map(|text| text.trim().to_owned())
            .filter(|text| !text.is_empty())
            .collect();
        if !matches!(cleaned.as_slice(), [only] if only.as_str() == "LITELLM ESCALATE") {
            payload.insert(
                "escalation_keywords".to_owned(),
                Json::List(cleaned.into_iter().map(Json::Text).collect()),
            );
        }
    }
    Some(())
}

#[derive(Clone, Copy)]
enum Numbers {
    Float,
    Int,
}

fn converted_list(value: &Json, convert: fn(&Json) -> Option<Json>) -> Option<Json> {
    let Json::List(items) = value else {
        return None;
    };
    let mut converted = Vec::with_capacity(items.len());
    for item in items {
        converted.push(convert(item)?);
    }
    Some(Json::List(converted))
}

fn number_map(value: &Json, numbers: Numbers) -> Option<Json> {
    let Json::Object(declared) = value else {
        return None;
    };
    let mut entries: BTreeMap<String, Json> = BTreeMap::new();
    for (key, entry) in declared {
        let Json::Number(number) = entry else {
            return None;
        };
        let converted = match numbers {
            Numbers::Float => Number::from_f64(number.as_f64()?)?,
            Numbers::Int => Number::from(number.as_i64()?),
        };
        entries.insert(key.clone(), Json::Number(converted));
    }
    Some(Json::Object(entries.into_iter().collect()))
}

fn float_option(value: &Json) -> Option<Json> {
    match value {
        Json::Null => Some(Json::Null),
        Json::Number(number) => Some(Json::Number(Number::from_f64(number.as_f64()?)?)),
        Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
    }
}

fn dimension_payload(entry: &Json) -> Option<Json> {
    let Json::Object(fields) = entry else {
        return None;
    };
    let name = text_value(fields.get("name")?)?;
    let Json::Number(weight) = fields.get("weight")? else {
        return None;
    };
    let weight_number = Number::from_f64(weight.as_f64()?)?;
    let keywords = match text_list(fields.get("keywords"))? {
        TextList::Absent => Vec::new(),
        TextList::Values(texts) => texts,
        TextList::Null => return None,
    };
    let patterns = match text_list(fields.get("patterns"))? {
        TextList::Absent => Vec::new(),
        TextList::Values(texts) => texts,
        TextList::Null => return None,
    };
    let scoring_mode = dimension_scoring_mode(fields)?;
    let mut object: BTreeMap<String, Json> = BTreeMap::new();
    object.insert(
        "keywords".to_owned(),
        Json::List(keywords.into_iter().map(Json::Text).collect()),
    );
    object.insert("name".to_owned(), Json::Text(name));
    object.insert(
        "patterns".to_owned(),
        Json::List(patterns.into_iter().map(Json::Text).collect()),
    );
    if scoring_mode == "match_count" {
        object.insert("scoring_mode".to_owned(), Json::Text(scoring_mode));
    }
    object.insert("weight".to_owned(), Json::Number(weight_number));
    Some(Json::Object(object.into_iter().collect()))
}

// the section's ScoringMode enum admits binary and match_count, so a text is one of those two
fn dimension_scoring_mode(fields: &JsonObject) -> Option<String> {
    match fields.get("scoring_mode") {
        None => Some("binary".to_owned()),
        Some(Json::Text(text)) => Some(text.clone()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            None
        }
    }
}

fn keyword_rule(entry: &Json) -> Option<Json> {
    let Json::Object(fields) = entry else {
        return None;
    };
    let declared = match text_list(fields.get("keywords"))? {
        TextList::Values(texts) => texts,
        TextList::Absent | TextList::Null => return None,
    };
    let mut keywords = Vec::with_capacity(declared.len());
    for keyword in declared {
        let stripped = keyword.trim();
        if !stripped.is_empty() {
            keywords.push(stripped.to_owned());
        }
    }
    if keywords.is_empty() {
        return None;
    }
    let tier = text_value(fields.get("tier")?)?;
    let mut object = IndexMap::new();
    object.insert(
        "keywords".to_owned(),
        Json::List(keywords.into_iter().map(Json::Text).collect()),
    );
    object.insert("tier".to_owned(), Json::Text(tier.trim().to_owned()));
    Some(Json::Object(object))
}

fn text_value(value: &Json) -> Option<String> {
    match value {
        Json::Text(text) => Some(text.clone()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

enum TextList {
    Absent,
    Null,
    Values(Vec<String>),
}

fn text_list(value: Option<&Json>) -> Option<TextList> {
    match value {
        None => Some(TextList::Absent),
        Some(Json::Null) => Some(TextList::Null),
        Some(Json::List(items)) => {
            let mut texts = Vec::with_capacity(items.len());
            for item in items {
                let Json::Text(text) = item else {
                    return None;
                };
                texts.push(text.clone());
            }
            Some(TextList::Values(texts))
        }
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => None,
    }
}
