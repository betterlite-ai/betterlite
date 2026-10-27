use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::error::{ContentBlocked, NestedPayload};

mod tests;

pub const MAX_STRUCTURED_CONTENT_SCAN_DEPTH: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum PathPart {
    Key(String),
    Index(usize),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct LeafPath(pub Vec<PathPart>);

impl LeafPath {
    #[must_use]
    pub fn dotted(&self) -> String {
        let mut parts = Vec::with_capacity(self.0.len());
        for part in &self.0 {
            match part {
                PathPart::Key(key) => parts.push(key.clone()),
                PathPart::Index(index) => parts.push(index.to_string()),
            }
        }
        parts.join(".")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Leaf {
    pub path: LeafPath,
    pub text: String,
}

// labels are object keys and numbers: a guardrail may match them but nothing can rewrite them
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JsonScan {
    pub leaves: Vec<Leaf>,
    pub labels: Vec<String>,
}

pub fn scan_json(
    value: &GuardrailValue,
    payload: NestedPayload,
) -> Result<JsonScan, ContentBlocked> {
    let mut found = JsonScan::default();
    collect(value, &mut Vec::new(), &mut found, payload)?;
    Ok(found)
}

fn collect(
    value: &GuardrailValue,
    path: &mut Vec<PathPart>,
    found: &mut JsonScan,
    payload: NestedPayload,
) -> Result<(), ContentBlocked> {
    if path.len() > MAX_STRUCTURED_CONTENT_SCAN_DEPTH {
        return Err(ContentBlocked::TooDeep { payload });
    }
    match value {
        GuardrailValue::Text(text) => found.leaves.push(Leaf {
            path: LeafPath(path.clone()),
            text: text.clone(),
        }),
        GuardrailValue::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                path.push(PathPart::Index(index));
                collect(item, path, found, payload)?;
                path.pop();
            }
        }
        GuardrailValue::Object(entries) => {
            found.labels.extend(entries.keys().cloned());
            for (key, item) in entries {
                path.push(PathPart::Key(key.clone()));
                collect(item, path, found, payload)?;
                path.pop();
            }
        }
        GuardrailValue::SignedInteger(number) => found.labels.push(number.to_string()),
        GuardrailValue::UnsignedInteger(number) => found.labels.push(number.to_string()),
        GuardrailValue::Decimal(number) => found.labels.push(format!("{number:?}")),
        GuardrailValue::Null | GuardrailValue::Boolean(_) => {}
    }
    Ok(())
}
