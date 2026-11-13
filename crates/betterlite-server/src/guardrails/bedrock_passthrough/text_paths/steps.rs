use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathStep {
    Key(String),
    Index(usize),
}

// LiteLLM writes through the (container, key) references it collected; Rust records the route to them instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextPath(pub Vec<PathStep>);

pub(super) fn child_path(base: &[PathStep], keys: &[&str]) -> Vec<PathStep> {
    let mut path = base.to_vec();
    for key in keys {
        path.push(PathStep::Key((*key).to_owned()));
    }
    path
}

pub(super) fn text_at_path_mut<'root>(
    root: &'root mut GuardrailValue,
    path: &[PathStep],
) -> Option<&'root mut String> {
    let mut current = root;
    for step in path {
        current = match (current, step) {
            (GuardrailValue::Object(fields), PathStep::Key(key)) => fields.get_mut(key)?,
            (GuardrailValue::Array(items), PathStep::Index(index)) => items.get_mut(*index)?,
            _ => return None,
        };
    }
    let GuardrailValue::Text(text) = current else {
        return None;
    };
    Some(text)
}
