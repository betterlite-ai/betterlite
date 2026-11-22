use std::collections::BTreeMap;
use std::str::FromStr as _;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive as _, ToPrimitive as _};
use serde_json::Number;

// tuning_fingerprint's payload: the supplied heuristic-v1 fields after LiteLLM's model validators, as
// ComplexityRouterConfig.model_dump(mode="json", include=..., exclude=...) produces them
pub(super) fn payload(fields: &JsonObject) -> Option<IndexMap<String, Json>> {
    let mut payload = IndexMap::new();
    insert_float_map(&mut payload, fields, "tier_boundaries")?;
    insert_float_option(&mut payload, fields, "reasoning_override_min_score")?;
    insert_int_map(&mut payload, fields, "token_thresholds")?;
    insert_float_map(&mut payload, fields, "dimension_weights")?;
    insert_dimensions(&mut payload, fields)?;
    for name in [
        "code_keywords",
        "reasoning_keywords",
        "technical_keywords",
        "custom_technical_keywords",
        "simple_keywords",
    ] {
        insert_text_list(&mut payload, fields, name)?;
    }
    insert_escalation(&mut payload, fields)?;
    insert_keyword_rules(&mut payload, fields)?;
    payload.sort_keys();
    Some(payload)
}

fn insert_float_map(
    payload: &mut IndexMap<String, Json>,
    fields: &JsonObject,
    name: &str,
) -> Option<()> {
    let Some(value) = fields.get(name) else {
        return Some(());
    };
    let Json::Object(declared) = value else {
        return None;
    };
    let mut entries: BTreeMap<String, Json> = BTreeMap::new();
    for (key, entry) in declared {
        entries.insert(key.clone(), float_json(entry)?);
    }
    payload.insert(name.to_owned(), Json::Object(entries.into_iter().collect()));
    Some(())
}

fn insert_int_map(
    payload: &mut IndexMap<String, Json>,
    fields: &JsonObject,
    name: &str,
) -> Option<()> {
    let Some(value) = fields.get(name) else {
        return Some(());
    };
    let Json::Object(declared) = value else {
        return None;
    };
    let mut entries: BTreeMap<String, Json> = BTreeMap::new();
    for (key, entry) in declared {
        let Json::Number(number) = entry else {
            return None;
        };
        entries.insert(key.clone(), Json::Number(Number::from(integral(number)?)));
    }
    payload.insert(name.to_owned(), Json::Object(entries.into_iter().collect()));
    Some(())
}

fn insert_float_option(
    payload: &mut IndexMap<String, Json>,
    fields: &JsonObject,
    name: &str,
) -> Option<()> {
    let Some(value) = fields.get(name) else {
        return Some(());
    };
    let normalized = match value {
        Json::Null => Json::Null,
        Json::Number(number) => Json::Number(Number::from_f64(number.as_f64()?)?),
        Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => return None,
    };
    payload.insert(name.to_owned(), normalized);
    Some(())
}

fn insert_dimensions(payload: &mut IndexMap<String, Json>, fields: &JsonObject) -> Option<()> {
    let Some(value) = fields.get("custom_dimensions") else {
        return Some(());
    };
    let Json::List(items) = value else {
        return None;
    };
    let mut dimensions = Vec::with_capacity(items.len());
    for item in items {
        dimensions.push(dimension_payload(item)?);
    }
    payload.insert("custom_dimensions".to_owned(), Json::List(dimensions));
    Some(())
}

fn dimension_payload(entry: &Json) -> Option<Json> {
    let Json::Object(fields) = entry else {
        return None;
    };
    let name = text_value(fields.get("name")?)?;
    let Json::Number(weight) = fields.get("weight")? else {
        return None;
    };
    let weight = Number::from_f64(weight.as_f64()?)?;
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
    object.insert("keywords".to_owned(), text_list_json(keywords));
    object.insert("name".to_owned(), Json::Text(name));
    object.insert("patterns".to_owned(), text_list_json(patterns));
    if scoring_mode == "match_count" {
        object.insert("scoring_mode".to_owned(), Json::Text(scoring_mode));
    }
    object.insert("weight".to_owned(), Json::Number(weight));
    Some(Json::Object(object.into_iter().collect()))
}

fn dimension_scoring_mode(fields: &JsonObject) -> Option<String> {
    match fields.get("scoring_mode") {
        None => Some("binary".to_owned()),
        Some(Json::Text(text)) if text == "binary" || text == "match_count" => Some(text.clone()),
        Some(
            Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::List(_)
            | Json::Object(_)
            | Json::Text(_),
        ) => None,
    }
}

fn insert_text_list(
    payload: &mut IndexMap<String, Json>,
    fields: &JsonObject,
    name: &str,
) -> Option<()> {
    let Some(value) = fields.get(name) else {
        return Some(());
    };
    let normalized = match text_list(Some(value))? {
        TextList::Absent => return None,
        TextList::Null => Json::Null,
        TextList::Values(texts) => text_list_json(texts),
    };
    payload.insert(name.to_owned(), normalized);
    Some(())
}

fn insert_escalation(payload: &mut IndexMap<String, Json>, fields: &JsonObject) -> Option<()> {
    let Some(value) = fields.get("escalation_keywords") else {
        return Some(());
    };
    match text_list(Some(value))? {
        TextList::Absent => return None,
        TextList::Null => {
            payload.insert("escalation_keywords".to_owned(), Json::Null);
        }
        TextList::Values(texts) => {
            let mut cleaned = Vec::with_capacity(texts.len());
            for text in texts {
                let stripped = text.trim();
                if !stripped.is_empty() {
                    cleaned.push(stripped.to_owned());
                }
            }
            let default_marker =
                matches!(cleaned.as_slice(), [only] if only.as_str() == "LITELLM ESCALATE");
            if !default_marker {
                payload.insert("escalation_keywords".to_owned(), text_list_json(cleaned));
            }
        }
    }
    Some(())
}

fn insert_keyword_rules(payload: &mut IndexMap<String, Json>, fields: &JsonObject) -> Option<()> {
    let Some(value) = fields.get("keyword_tier_rules") else {
        return Some(());
    };
    let Json::List(items) = value else {
        return None;
    };
    let mut rules = Vec::with_capacity(items.len());
    for item in items {
        rules.push(keyword_rule(item)?);
    }
    payload.insert("keyword_tier_rules".to_owned(), Json::List(rules));
    Some(())
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
    object.insert("keywords".to_owned(), text_list_json(keywords));
    object.insert("tier".to_owned(), Json::Text(tier.trim().to_owned()));
    Some(Json::Object(object))
}

fn text_list_json(texts: Vec<String>) -> Json {
    Json::List(texts.into_iter().map(Json::Text).collect())
}

fn float_json(entry: &Json) -> Option<Json> {
    let Json::Number(number) = entry else {
        return None;
    };
    Number::from_f64(number.as_f64()?).map(Json::Number)
}

fn integral(number: &Number) -> Option<i64> {
    if let Some(value) = number.as_i64() {
        return Some(value);
    }
    let text = number.to_string();
    let scaled = Decimal::from_str(&text)
        .ok()
        .or_else(|| number.as_f64().and_then(Decimal::from_f64))?;
    if !scaled.fract().is_zero() {
        return None;
    }
    scaled.to_i64()
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
