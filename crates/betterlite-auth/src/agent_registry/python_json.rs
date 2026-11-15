use betterlite_models::decisions::Json;
use indexmap::IndexMap;

mod tests;

// Python's json.dumps(sort_keys=True) orders the keys of every object by code point, at every depth
pub(crate) fn sorted(value: &Json) -> Json {
    match value {
        Json::List(items) => Json::List(items.iter().map(sorted).collect()),
        Json::Object(fields) => Json::Object(sorted_object(fields)),
        other => other.to_owned(),
    }
}

fn sorted_object(fields: &IndexMap<String, Json>) -> IndexMap<String, Json> {
    let mut entries: Vec<(&String, &Json)> = fields.iter().collect();
    entries.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));
    entries
        .into_iter()
        .map(|(name, value)| (name.to_owned(), sorted(value)))
        .collect()
}
