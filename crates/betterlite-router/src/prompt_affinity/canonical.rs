use betterlite_logging::python_json;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

// json.dumps(value, sort_keys=True, separators=(",", ":")): the bytes LiteLLM hashes, keys sorted at every depth
#[must_use]
pub fn canonical_bytes(value: &Json) -> Vec<u8> {
    let mut out = String::new();
    write_value(&mut out, value);
    out.into_bytes()
}

#[must_use]
pub fn canonical_list_bytes(items: &[Json]) -> Vec<u8> {
    let mut out = String::new();
    write_list(&mut out, items);
    out.into_bytes()
}

fn write_value(out: &mut String, value: &Json) {
    match value {
        Json::List(items) => write_list(out, items),
        Json::Object(fields) => write_object(out, fields),
        // scalars keep Python's own json.dumps text, escapes and float repr included
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => {
            out.push_str(&python_json::dumps(value));
        }
    }
}

fn write_list(out: &mut String, items: &[Json]) {
    out.push('[');
    for (position, item) in items.iter().enumerate() {
        if position != 0 {
            out.push(',');
        }
        write_value(out, item);
    }
    out.push(']');
}

fn write_object(out: &mut String, fields: &IndexMap<String, Json>) {
    let mut sorted: Vec<(&String, &Json)> = fields.iter().collect();
    sorted.sort_unstable_by(|left, right| left.0.cmp(right.0));
    out.push('{');
    for (position, (key, item)) in sorted.into_iter().enumerate() {
        if position != 0 {
            out.push(',');
        }
        out.push_str(&python_json::dumps(&Json::Text(key.clone())));
        out.push(':');
        write_value(out, item);
    }
    out.push('}');
}
