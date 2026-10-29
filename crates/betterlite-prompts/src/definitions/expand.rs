use std::collections::BTreeSet;
use std::sync::Arc;

use super::error::DefinitionsError;
use super::estimate::estimate_json_bytes;
use crate::arguments::json::{Json, Object};

// A name resolves only against the local `defs` tables built while walking, so a
// `$ref` that is not present there (an external URL, a typo) is left untouched instead
// of ever being fetched.
pub fn unpack_defs(
    schema: Object,
    defs: &Object,
    max_inlined_bytes: Option<usize>,
) -> Result<Object, DefinitionsError> {
    let original = schema.clone();
    // Python merges the root's own `$defs` / `definitions` into `root_defs` before even
    // checking whether the root itself is a `$ref`; building the scope outside
    // `expand_tree` lets the root ref resolve against its own sibling defs.
    let root_scope = Scope::root(defs).descend(&schema);
    let root = expand_tree(Json::Object(schema), root_scope, max_inlined_bytes)?;
    Ok(match root {
        Json::Object(object) => object,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => original,
    })
}

#[derive(Clone)]
struct Scope {
    defs: Arc<Object>,
    chain: Arc<BTreeSet<String>>,
}

impl Scope {
    fn root(defs: &Object) -> Self {
        Self {
            defs: Arc::new(defs.clone()),
            chain: Arc::new(BTreeSet::new()),
        }
    }

    fn descend(&self, node: &Object) -> Self {
        Self {
            defs: merge_defs(&self.defs, node),
            chain: Arc::clone(&self.chain),
        }
    }

    fn follow(&self, name: &str, target: Option<&Object>) -> Self {
        let mut chain = (*self.chain).clone();
        chain.insert(name.to_owned());
        let defs = target.map_or_else(
            || Arc::clone(&self.defs),
            |object| merge_defs(&self.defs, object),
        );
        Self {
            defs,
            chain: Arc::new(chain),
        }
    }
}

fn merge_defs(active: &Arc<Object>, node: &Object) -> Arc<Object> {
    let own_defs = node.get("$defs").and_then(Json::as_object);
    let own_definitions = node.get("definitions").and_then(Json::as_object);
    if own_defs.is_none() && own_definitions.is_none() {
        return Arc::clone(active);
    }
    let mut merged = (**active).clone();
    for own in [own_defs, own_definitions].into_iter().flatten() {
        for (key, value) in own {
            merged.insert(key.clone(), value.clone());
        }
    }
    Arc::new(merged)
}

fn ref_name(value: &Json) -> Option<&str> {
    match value {
        Json::Text(text) => text.rsplit('/').next(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

enum Item {
    Visit { node: Json, scope: Scope },
    Collect(Label),
}

enum Label {
    Object(Vec<String>),
    List(usize),
}

enum Resolution {
    Expand { node: Json, scope: Scope },
    Unchanged(Json),
}

struct Budget {
    limit: Option<usize>,
    spent: usize,
}

impl Budget {
    const fn new(limit: Option<usize>) -> Self {
        Self { limit, spent: 0 }
    }

    const fn charge(&mut self, amount: usize) -> Result<(), DefinitionsError> {
        self.spent = self.spent.saturating_add(amount);
        match self.limit {
            Some(limit) if self.spent > limit => {
                Err(DefinitionsError::ExpansionBudgetExceeded { limit })
            }
            Some(_) | None => Ok(()),
        }
    }
}

// Walks `root` with an explicit work stack instead of native recursion, so a deeply
// nested schema (legitimate or adversarial) cannot exhaust the call stack.
fn expand_tree(
    root: Json,
    scope: Scope,
    max_inlined_bytes: Option<usize>,
) -> Result<Json, DefinitionsError> {
    let mut budget = Budget::new(max_inlined_bytes);
    let mut work = vec![Item::Visit { node: root, scope }];
    let mut output: Vec<Json> = Vec::new();
    while let Some(item) = work.pop() {
        match item {
            Item::Visit {
                node,
                scope: item_scope,
            } => {
                visit(node, &item_scope, &mut budget, &mut work, &mut output)?;
            }
            Item::Collect(label) => collect(label, &mut output),
        }
    }
    Ok(output.pop().unwrap_or(Json::Null))
}

fn visit(
    node: Json,
    scope: &Scope,
    budget: &mut Budget,
    work: &mut Vec<Item>,
    output: &mut Vec<Json>,
) -> Result<(), DefinitionsError> {
    match node {
        Json::Object(object) => {
            let Some(reference) = object.get("$ref").cloned() else {
                push_object_children(object, scope, work);
                return Ok(());
            };
            match resolve_ref(&reference, object, scope, budget)? {
                Resolution::Expand {
                    node: resolved,
                    scope: next_scope,
                } => work.push(Item::Visit {
                    node: resolved,
                    scope: next_scope,
                }),
                Resolution::Unchanged(unchanged) => output.push(unchanged),
            }
        }
        Json::List(items) => visit_list(items, scope, work),
        scalar @ (Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_)) => {
            output.push(scalar);
        }
    }
    Ok(())
}

fn resolve_ref(
    reference: &Json,
    object: Object,
    scope: &Scope,
    budget: &mut Budget,
) -> Result<Resolution, DefinitionsError> {
    let Some(name) = ref_name(reference) else {
        return Ok(Resolution::Unchanged(Json::Object(object)));
    };
    if scope.chain.contains(name) {
        return Ok(Resolution::Unchanged(Json::Object(object)));
    }
    let Some(target) = scope.defs.get(name) else {
        return Ok(Resolution::Unchanged(Json::Object(object)));
    };
    budget.charge(estimate_json_bytes(target))?;
    let resolved = target.clone();
    let next_scope = scope.follow(name, resolved.as_object());
    Ok(Resolution::Expand {
        node: resolved,
        scope: next_scope,
    })
}

fn push_object_children(object: Object, scope: &Scope, work: &mut Vec<Item>) {
    let local_scope = scope.descend(&object);
    let entries: Vec<(String, Json)> = object.into_iter().collect();
    let keys: Vec<String> = entries.iter().map(|(key, _)| key.clone()).collect();
    work.push(Item::Collect(Label::Object(keys)));
    for (_, value) in entries.into_iter().rev() {
        work.push(Item::Visit {
            node: value,
            scope: local_scope.clone(),
        });
    }
}

fn visit_list(items: Vec<Json>, scope: &Scope, work: &mut Vec<Item>) {
    work.push(Item::Collect(Label::List(items.len())));
    for item in items.into_iter().rev() {
        work.push(Item::Visit {
            node: item,
            scope: scope.clone(),
        });
    }
}

fn collect(label: Label, output: &mut Vec<Json>) {
    match label {
        Label::Object(keys) => {
            let mut pairs: Vec<(String, Json)> = Vec::with_capacity(keys.len());
            for key in keys.into_iter().rev() {
                pairs.push((key, output.pop().unwrap_or(Json::Null)));
            }
            pairs.reverse();
            output.push(Json::Object(pairs.into_iter().collect()));
        }
        Label::List(length) => {
            let mut items: Vec<Json> = Vec::with_capacity(length);
            for _ in 0..length {
                items.push(output.pop().unwrap_or(Json::Null));
            }
            items.reverse();
            output.push(Json::List(items));
        }
    }
}

mod tests;
