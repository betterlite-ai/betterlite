use crate::arguments::json::Json;

// A byte count, not an exact serialised size: a schema-bomb budget only needs a value within a small constant factor, so this walks the tree once instead of serialising it.
#[must_use]
pub fn estimate_json_bytes(value: &Json) -> usize {
    let mut total: usize = 0;
    let mut stack: Vec<&Json> = vec![value];
    while let Some(node) = stack.pop() {
        total = total.saturating_add(match node {
            Json::Object(object) => {
                stack.extend(object.values());
                object.keys().fold(2, |sum, key| {
                    sum.saturating_add(key.len()).saturating_add(4)
                })
            }
            Json::List(items) => {
                stack.extend(items.iter());
                items.len().saturating_sub(1).saturating_add(2)
            }
            Json::Text(text) => text.len().saturating_add(2),
            Json::Bool(true) | Json::Null => 4,
            Json::Bool(false) => 5,
            Json::Number(_) => 24,
        });
    }
    total
}

mod tests;
