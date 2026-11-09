use std::str::FromStr as _;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

// Above this nesting the parser recurses deeper than a gateway stack should, so the literal is refused.
pub const MAX_LITERAL_DEPTH: usize = 128;

// Python's ast.literal_eval, as LiteLLM reads a captured response string back.
#[must_use]
pub fn literal_json(text: &str) -> Option<Json> {
    if nesting_depth(text) > MAX_LITERAL_DEPTH {
        return None;
    }
    let literal = text.parse::<py_literal::Value>().ok()?;
    literal_to_json(&literal)
}

fn literal_to_json(literal: &py_literal::Value) -> Option<Json> {
    match literal {
        py_literal::Value::None => Some(Json::Null),
        py_literal::Value::Boolean(flag) => Some(Json::Bool(*flag)),
        py_literal::Value::String(text) => Some(Json::Text(text.clone())),
        py_literal::Value::Integer(integer) => Number::from_str(&integer.to_string())
            .ok()
            .map(Json::Number),
        py_literal::Value::Float(float) => Number::from_f64(*float).map(Json::Number),
        py_literal::Value::List(items)
        | py_literal::Value::Tuple(items)
        | py_literal::Value::Set(items) => items
            .iter()
            .map(literal_to_json)
            .collect::<Option<Vec<Json>>>()
            .map(Json::List),
        py_literal::Value::Dict(entries) => dictionary(entries),
        py_literal::Value::Bytes(_) | py_literal::Value::Complex(_) => None,
    }
}

fn dictionary(entries: &[(py_literal::Value, py_literal::Value)]) -> Option<Json> {
    let mut fields = IndexMap::new();
    for (key, value) in entries {
        let py_literal::Value::String(name) = key else {
            return None;
        };
        fields.insert(name.clone(), literal_to_json(value)?);
    }
    Some(Json::Object(fields))
}

// Counts bracket nesting outside quoted strings, so the parser never sees an unbounded stack.
fn nesting_depth(text: &str) -> usize {
    let mut depth = 0_usize;
    let mut deepest = 0_usize;
    let mut quoted: Option<char> = None;
    let mut escaped = false;
    for character in text.chars() {
        if let Some(active) = quoted {
            match (escaped, character) {
                (true, _) => escaped = false,
                (false, '\\') => escaped = true,
                (false, closing) if closing == active => quoted = None,
                (false, _) => {}
            }
            continue;
        }
        match character {
            '\'' | '"' => quoted = Some(character),
            '(' | '[' | '{' => {
                depth = depth.saturating_add(1);
                deepest = deepest.max(depth);
            }
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    deepest
}
