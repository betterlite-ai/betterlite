use std::collections::BTreeSet;

use serde_json::{Map, Value};

use super::{REDACTED, SensitiveDataMasker, number};

pub(super) enum Kind<'fields> {
    Config,
    Payload,
    Redact,
    Exact(&'fields BTreeSet<String>),
}

#[derive(Clone, Copy)]
pub(super) enum Context {
    Root,
    Field(bool),
    Item(bool),
}

pub(super) struct Walker<'input> {
    pub masker: &'input SensitiveDataMasker,
    pub kind: Kind<'input>,
    pub excluded_keys: &'input [String],
    pub max_depth: usize,
    pub remaining: usize,
}

impl Walker<'_> {
    fn charge(&mut self, bytes: usize) -> Option<()> {
        self.remaining = self.remaining.checked_sub(bytes)?;
        Some(())
    }

    pub(super) fn walk(&mut self, node: &Value, depth: usize, context: Context) -> Option<Value> {
        if matches!(self.kind, Kind::Redact)
            && matches!(context, Context::Field(true))
            && !node.is_null()
        {
            self.charge(REDACTED.len())?;
            return Some(Value::String(REDACTED.to_owned()));
        }
        match node {
            Value::Object(mapping) => self.mapping(mapping, depth),
            Value::Array(items) => self.sequence(items, depth, context),
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
                self.charge(1)?;
                if let Value::String(text) = node {
                    self.charge(text.len())?;
                }
                if let Value::Number(number) = node {
                    self.charge(number.as_str().len())?;
                }
                let output = self.scalar(node, context);
                if let Value::String(text) = &output {
                    self.charge(text.len())?;
                }
                if let Value::Number(number) = &output {
                    self.charge(number.as_str().len())?;
                }
                Some(output)
            }
        }
    }

    pub(super) fn mapping(&mut self, data: &Map<String, Value>, depth: usize) -> Option<Value> {
        self.charge(1)?;
        if depth >= self.max_depth {
            self.charge(REDACTED.len())?;
            return Some(Value::String(REDACTED.to_owned()));
        }
        let mut output = Map::new();
        for (key, value) in data {
            self.charge(key.len())?;
            let sensitive = match &self.kind {
                Kind::Exact(fields) => depth == 0 && fields.contains(key),
                Kind::Config | Kind::Payload | Kind::Redact => {
                    self.masker.is_sensitive_key(key, self.excluded_keys)
                }
            };
            let masked = self.walk(value, depth.saturating_add(1), Context::Field(sensitive))?;
            output.insert(key.clone(), masked);
        }
        Some(Value::Object(output))
    }

    fn sequence(&mut self, items: &[Value], depth: usize, context: Context) -> Option<Value> {
        self.charge(1)?;
        if depth >= self.max_depth {
            self.charge(REDACTED.len())?;
            return Some(Value::String(REDACTED.to_owned()));
        }
        let sensitive = matches!(context, Context::Field(true) | Context::Item(true));
        let mut output = Vec::new();
        for item in items {
            output.push(self.walk(item, depth.saturating_add(1), Context::Item(sensitive))?);
        }
        Some(Value::Array(output))
    }

    fn scalar(&self, node: &Value, context: Context) -> Value {
        let sensitive = matches!(context, Context::Field(true) | Context::Item(true));
        match (&self.kind, context, node) {
            (Kind::Config, Context::Field(true), _) => {
                let text = match node {
                    Value::Null | Value::Array(_) | Value::Object(_) => String::new(),
                    Value::Bool(true) => "True".to_owned(),
                    Value::Bool(false) => "False".to_owned(),
                    Value::Number(value) => number::text(value),
                    Value::String(text) => text.clone(),
                };
                Value::String(self.masker.mask_value(&text))
            }
            (Kind::Config, Context::Field(_) | Context::Item(_), Value::Null) => {
                Value::String("None".to_owned())
            }
            (Kind::Config | Kind::Payload, _, Value::String(text)) if sensitive => {
                Value::String(self.masker.mask_value(text))
            }
            (Kind::Exact(_), Context::Field(true), Value::String(text)) => {
                Value::String(self.masker.mask_value(text))
            }
            (Kind::Config | Kind::Payload | Kind::Redact | Kind::Exact(_), _, _) => node.clone(),
        }
    }
}

mod tests;
