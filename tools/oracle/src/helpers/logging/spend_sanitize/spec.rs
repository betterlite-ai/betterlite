use std::collections::BTreeMap;

use betterlite_logging::spend_sanitize::value::{NodeId, SpendList, SpendMapping, SpendValue};
use indexmap::IndexMap;
use serde_json::{Map, Value};

// The case describes Python objects, so a repeated id is a shared object or a cycle, not a tree
#[derive(Default)]
pub(crate) struct Decoder {
    next: u64,
    named: BTreeMap<u64, NodeId>,
    decoded: BTreeMap<u64, SpendValue>,
}

impl Decoder {
    pub(crate) fn decode(&mut self, value: &Value) -> Result<SpendValue, String> {
        match value {
            Value::Null => Ok(SpendValue::Null),
            Value::Bool(flag) => Ok(SpendValue::Bool(*flag)),
            Value::Number(number) => Ok(SpendValue::Number(number.clone())),
            Value::String(text) => Ok(SpendValue::Text(text.clone())),
            Value::Array(items) => {
                let node = self.fresh();
                let decoded = items
                    .iter()
                    .map(|item| self.decode(item))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(SpendValue::List(SpendList {
                    node,
                    items: decoded,
                }))
            }
            Value::Object(entries) => self.node(entries),
        }
    }

    // A node names itself with "$py"; an id lets another node reference the same object
    fn node(&mut self, entries: &Map<String, Value>) -> Result<SpendValue, String> {
        let kind = entries.get("$py").and_then(Value::as_str);
        let raw_id = match kind {
            Some(_) => entries.get("id").and_then(Value::as_u64),
            None => None,
        };
        if kind == Some("ref") {
            let target = raw_id.ok_or_else(|| "a ref needs an integer id".to_owned())?;
            let node = self.node_for(target);
            let empty_kind = entries
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("mapping");
            return self
                .decoded
                .get(&target)
                .cloned()
                .map_or_else(|| empty_node(empty_kind, node), Ok);
        }
        let node = match raw_id {
            Some(id) => self.node_for(id),
            None => self.fresh(),
        };
        let value = match kind {
            Some("list") => {
                let items = entries
                    .get("items")
                    .and_then(Value::as_array)
                    .ok_or_else(|| "a list node needs items".to_owned())?;
                self.list_node(items, node)?
            }
            _ => self.mapping_node(entries, kind, node)?,
        };
        self.remember(raw_id, &value);
        Ok(value)
    }

    fn list_node(&mut self, items: &[Value], node: NodeId) -> Result<SpendValue, String> {
        let mut decoded = Vec::with_capacity(items.len());
        for item in items {
            decoded.push(self.decode(item)?);
        }
        Ok(SpendValue::List(SpendList {
            node,
            items: decoded,
        }))
    }

    fn mapping_node(
        &mut self,
        entries: &Map<String, Value>,
        kind: Option<&str>,
        node: NodeId,
    ) -> Result<SpendValue, String> {
        let fields = match kind {
            None => entries,
            Some("mapping" | "model" | "object") => entries
                .get("entries")
                .and_then(Value::as_object)
                .ok_or_else(|| "a mapping node needs entries".to_owned())?,
            Some(other) => return Err(format!("unknown $py kind {other}")),
        };
        let mapping = SpendMapping {
            node,
            entries: self.entries_of(fields)?,
        };
        match kind {
            Some("model") => Ok(SpendValue::Model(mapping)),
            Some("object") => Ok(SpendValue::Object(mapping)),
            _ => Ok(SpendValue::Mapping(mapping)),
        }
    }

    fn entries_of(
        &mut self,
        fields: &Map<String, Value>,
    ) -> Result<IndexMap<String, SpendValue>, String> {
        let mut decoded = IndexMap::new();
        for (key, item) in fields {
            decoded.insert(key.clone(), self.decode(item)?);
        }
        Ok(decoded)
    }

    fn remember(&mut self, raw_id: Option<u64>, value: &SpendValue) {
        if let Some(id) = raw_id {
            self.decoded.insert(id, value.clone());
        }
    }

    fn node_for(&mut self, raw_id: u64) -> NodeId {
        if let Some(&node) = self.named.get(&raw_id) {
            return node;
        }
        let node = self.fresh();
        self.named.insert(raw_id, node);
        node
    }

    fn fresh(&mut self) -> NodeId {
        let node = NodeId::from(self.next);
        self.next = self.next.saturating_add(1);
        node
    }
}

// A reference into a node still being built can never read its content, so its kind is enough
fn empty_node(kind: &str, node: NodeId) -> Result<SpendValue, String> {
    match kind {
        "mapping" => Ok(SpendValue::Mapping(SpendMapping {
            node,
            entries: IndexMap::new(),
        })),
        "model" => Ok(SpendValue::Model(SpendMapping {
            node,
            entries: IndexMap::new(),
        })),
        "object" => Ok(SpendValue::Object(SpendMapping {
            node,
            entries: IndexMap::new(),
        })),
        "list" => Ok(SpendValue::List(SpendList {
            node,
            items: Vec::new(),
        })),
        other => Err(format!("unknown ref kind {other}")),
    }
}
